//! Resolve Page-owned triggers to one server-authorized workflow entry.
//!
//! `PageTrigger` is request data carried beside the caller's normal
//! authentication. A dynamic action capability is deliberately verified here
//! and never enters the authentication middleware.

use std::{collections::HashSet, future::Future};

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
    execution::{
        DYNAMIC_PAGE_ACTION_ID_PREFIX, PageActionClaims, variant, variant::ResolvedTarget,
        verify_page_action_capability,
    },
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

impl PageTrigger {
    /// The Page contract revision the caller's bootstrap reported.
    pub fn manifest_revision(&self) -> Option<&str> {
        match self {
            PageTrigger::Action {
                manifest_revision, ..
            }
            | PageTrigger::Special {
                manifest_revision, ..
            } => manifest_revision.as_deref(),
        }
    }
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
    /// The Event-level target this trigger resolved within: the primary or the
    /// Live page variant a bootstrap served. Dispatch applies its board,
    /// version, page and variables overlay and tags the run row with it.
    pub target: ResolvedTarget,
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
///
/// `variant_pin` is the served variant a bootstrap reported (`stable` or a Live
/// variant name). A compiled trigger resolves against that target's contract.
/// Without a pin the target is inferred from the manifest revision the request
/// carries — see [`infer_compiled_target`]. A dynamic capability is bound by
/// its own sealed claims instead; a pin that disagrees with them is refused.
pub async fn resolve_page_trigger(
    state: &AppState,
    permission: &AppPermissionResponse,
    app_id: &str,
    event: &Event,
    trigger: &PageTrigger,
    variant_pin: Option<&str>,
) -> Result<ResolvedPageTrigger, ApiError> {
    if !permission.has_permission(RolePermissions::ExecuteEvents) {
        return Err(ApiError::forbidden(
            "Page execution requires Event runtime permission",
        ));
    }
    if !event.active {
        return Err(ApiError::forbidden("The Page Event is not active"));
    }
    if event.default_page_id.is_none() {
        return Err(ApiError::bad_request(
            "Page triggers can only invoke Events that own a Page",
        ));
    }

    let pinned_target = pinned_page_target(event, variant_pin)?;
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
            pinned_target,
            action_id,
            capability_jwt,
        )
        .await;
    }
    let (target, contract) = match pinned_target {
        Some(target) => {
            let contract = resolve_target_contract(state, app_id, event, &target).await?;
            (target, contract)
        }
        None => {
            infer_compiled_target(event, trigger.manifest_revision(), |target| async move {
                resolve_target_contract(state, app_id, event, &target).await
            })
            .await?
        }
    };
    let event = &variant::apply_target(event.clone(), &target);
    let configured_page_id = event.default_page_id.as_deref().ok_or_else(|| {
        ApiError::bad_request("Page triggers can only invoke Events that own a Page")
    })?;
    require_compiled_revision(trigger.manifest_revision(), &contract.manifest_revision)?;
    let page_execution = &contract.page_execution;
    let node_id = match trigger {
        PageTrigger::Action {
            action_id,
            capability_jwt,
            ..
        } => resolve_action(page_execution, action_id, capability_jwt.as_deref())?,
        PageTrigger::Special { special_event, .. } => {
            resolve_special(page_execution, *special_event)?
        }
    };

    if !contract.entry_node_ids.contains(&node_id) {
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
        target,
    })
}

/// The explicitly pinned page target, or `None` when the request carried no
/// pin. An unknown or non-Live pin is a bad request, like on every other
/// invoke surface.
fn pinned_page_target(
    event: &Event,
    variant_pin: Option<&str>,
) -> Result<Option<ResolvedTarget>, ApiError> {
    variant::explicit_page_target(event, variant_pin).map_err(|error| {
        ApiError::bad_request(format!(
            "{error}; the served variant may have been removed — reload the Page"
        ))
    })
}

/// The governed contract of one page target: the Event as that target sees it,
/// compiled through [`resolve_page_contract`].
async fn resolve_target_contract(
    state: &AppState,
    app_id: &str,
    event: &Event,
    target: &ResolvedTarget,
) -> Result<ResolvedPageContract, ApiError> {
    let event = variant::apply_target(event.clone(), target);
    resolve_page_contract(state, app_id, &event).await
}

/// The page target an unpinned compiled trigger addresses.
///
/// A viewer served a Live variant at bootstrap may send no pin, but the
/// trigger still carries that contract's manifest revision, so the revision
/// identifies the target: the primary when its contract matches, else the
/// unique Live page variant whose contract does. Variant contracts are only
/// compiled when the primary did not match. No match, more than one match, a
/// variant that fails to compile, or a missing revision all keep the primary —
/// [`require_compiled_revision`] then judges the revision itself.
async fn infer_compiled_target<F, Fut>(
    event: &Event,
    requested_revision: Option<&str>,
    mut resolve_contract: F,
) -> Result<(ResolvedTarget, ResolvedPageContract), ApiError>
where
    F: FnMut(ResolvedTarget) -> Fut,
    Fut: Future<Output = Result<ResolvedPageContract, ApiError>>,
{
    let primary = ResolvedTarget::primary(event);
    let primary_contract = resolve_contract(primary.clone()).await?;
    let Some(requested) = requested_revision
        .map(str::trim)
        .filter(|revision| !revision.is_empty())
    else {
        return Ok((primary, primary_contract));
    };
    if primary_contract.manifest_revision == requested {
        return Ok((primary, primary_contract));
    }

    let mut matched: Option<(ResolvedTarget, ResolvedPageContract)> = None;
    for target in variant::page_targets(event)
        .into_iter()
        .filter(|target| target.variant_name.is_some())
    {
        let contract = match resolve_contract(target.clone()).await {
            Ok(contract) => contract,
            Err(error) => {
                tracing::debug!(
                    event_id = %event.id,
                    variant = ?target.variant_name,
                    error = %error,
                    "Page variant contract did not compile while inferring the compiled trigger target"
                );
                continue;
            }
        };
        if contract.manifest_revision != requested {
            continue;
        }
        if matched.is_some() {
            tracing::warn!(
                event_id = %event.id,
                requested,
                "Page manifest revision names more than one Live variant; resolving against the primary"
            );
            return Ok((primary, primary_contract));
        }
        matched = Some((target, contract));
    }
    Ok(matched.unwrap_or((primary, primary_contract)))
}

/// The one legal page target a sealed capability names: the configured target
/// whose `(page_id, board_id, board selector)` triple the claims carry. A
/// variant removed since the session was minted has no match and fails closed.
fn sealed_page_target(
    event: &Event,
    claims: &PageActionClaims,
) -> Result<ResolvedTarget, ApiError> {
    let board_etag = claims
        .target_board_etag
        .as_deref()
        .map(str::trim)
        .filter(|etag| !etag.is_empty());
    variant::page_targets(event)
        .into_iter()
        .find(|target| {
            target.default_page_id.as_deref() == Some(claims.source_page_id.as_str())
                && target.board_id == claims.target_board_id
                && dynamic_capability_board_selector_matches(
                    target.board_version,
                    claims.target_board_version,
                    board_etag,
                )
        })
        .ok_or_else(|| {
            ApiError::forbidden(
                "The Page action capability no longer matches a configured target of this Event; the variant it was served from may have been removed — reload the Page",
            )
        })
}

async fn resolve_dynamic_page_trigger(
    state: &AppState,
    permission: &AppPermissionResponse,
    app_id: &str,
    event: &Event,
    pinned_target: Option<ResolvedTarget>,
    action_id: &str,
    capability_jwt: &str,
) -> Result<ResolvedPageTrigger, ApiError> {
    let claims = verify_page_action_capability(capability_jwt)
        .map_err(|_| ApiError::forbidden("The Page action capability is invalid"))?;
    let caller_sub = permission.effective_user_id().map_err(|_| {
        ApiError::forbidden("Page execution requires a caller linked to a user account")
    })?;
    let target = sealed_page_target(event, &claims)?;
    if pinned_target.is_some_and(|pinned| pinned.variant_name != target.variant_name) {
        return Err(ApiError::forbidden(
            "The Page action capability was sealed for a different variant than the one requested; reload the Page",
        ));
    }
    let page_id = target.default_page_id.as_deref().ok_or_else(|| {
        ApiError::bad_request("Page triggers can only invoke Events that own a Page")
    })?;
    let binding = DynamicCapabilityBinding {
        caller_sub: &caller_sub,
        caller_technical_user: permission.technical_user_id(),
        app_id,
        event_id: &event.id,
        page_id,
        board_id: &target.board_id,
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
    let manifest = load_exact_prerun_manifest(
        state,
        app_id,
        &target.board_id,
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
            .master_board_shared(app_id, &target.board_id, state, claims.target_board_version)
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
        target,
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

/// A pinned target accepts only its exact version; an ETag-bound Latest target
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

/// Revision gate for targets compiled out of the Page itself.
///
/// The revision hashes the whole Board, so any unrelated Board edit supersedes
/// every already-rendered Page. For a compiled target it is not an
/// authorization signal: [`resolve_action`] and [`resolve_special`] both look
/// the target up in the *current* contract, and `entry_node_ids` gates what may
/// run. A superseded revision therefore resolves normally; an absent one still
/// fails, because the caller must have come through a real bootstrap. Clients
/// re-stamp a compiled trigger with the revision a prerun just reported, so a
/// first-party caller normally arrives current anyway.
///
/// A dynamic capability is NOT gated here and never reaches this function:
/// [`resolve_dynamic_page_trigger`] discards the requested revision entirely
/// and binds on the capability JWT's own claims instead.
fn require_compiled_revision(requested: Option<&str>, current: &str) -> Result<(), ApiError> {
    let requested = requested
        .filter(|revision| !revision.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("The Page manifest revision is required"))?;
    if requested != current {
        tracing::debug!(
            requested,
            current,
            "Page manifest was superseded; resolving the compiled target against the current contract"
        );
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
    use flow_like::flow::event::{EventVariant, EventVariantMode};

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

    fn page_event() -> Event {
        Event {
            id: "event-1".to_string(),
            name: "Page".to_string(),
            description: String::new(),
            board_id: "board-1".to_string(),
            board_version: Some((1, 2, 3)),
            node_id: "node-1".to_string(),
            variables: std::collections::HashMap::new(),
            config: Vec::new(),
            active: true,
            canary: None,
            variants: Vec::new(),
            priority: 0,
            event_type: "generic_form".to_string(),
            notes: None,
            event_version: (1, 0, 0),
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
            default_page_id: Some("page-1".to_string()),
            inputs: Vec::new(),
            route: Some("/".to_string()),
            is_default: true,
            execution_mode: Default::default(),
            exposure: Default::default(),
            correlation_mappings: None,
        }
    }

    fn page_variant(
        name: &str,
        board_id: &str,
        board_version: Option<(u32, u32, u32)>,
    ) -> EventVariant {
        EventVariant {
            name: name.to_string(),
            board_id: board_id.to_string(),
            board_version,
            node_id: "node-canary".to_string(),
            variables: std::collections::HashMap::new(),
            default_page_id: Some("page-canary".to_string()),
            mode: EventVariantMode::Live { weight: 0.5 },
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    fn variant_claims() -> PageActionClaims {
        let mut claims = matching_dynamic_claims();
        claims.source_page_id = "page-canary".into();
        claims.target_board_id = "board-canary".into();
        claims.target_board_version = Some((2, 0, 0));
        claims
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
    fn sealed_claims_bind_to_the_primary_or_a_configured_page_variant_only() {
        let mut event = page_event();
        event.variants = vec![page_variant("canary", "board-canary", Some((2, 0, 0)))];

        let primary = sealed_page_target(&event, &matching_dynamic_claims()).unwrap();
        assert_eq!(primary.variant_name, None);
        assert_eq!(primary.board_id, "board-1");
        assert_eq!(primary.default_page_id.as_deref(), Some("page-1"));

        let canary = sealed_page_target(&event, &variant_claims()).unwrap();
        assert_eq!(canary.variant_name.as_deref(), Some("canary"));
        assert_eq!(canary.board_id, "board-canary");
        assert_eq!(canary.board_version, Some((2, 0, 0)));
        assert_eq!(canary.default_page_id.as_deref(), Some("page-canary"));

        // Any triple outside {primary, configured Live variants} fails closed:
        // the variant's page on the primary board, the primary's page on the
        // variant board, a foreign version, and a selector of the wrong kind.
        let cases = [
            ("variant page on the primary board", {
                let mut claims = matching_dynamic_claims();
                claims.source_page_id = "page-canary".into();
                claims
            }),
            ("primary page on the variant board", {
                let mut claims = variant_claims();
                claims.source_page_id = "page-1".into();
                claims
            }),
            ("unconfigured version", {
                let mut claims = variant_claims();
                claims.target_board_version = Some((2, 0, 1));
                claims
            }),
            ("etag selector on a pinned variant", {
                let mut claims = variant_claims();
                claims.target_board_version = None;
                claims.target_board_etag = Some("etag-1".into());
                claims
            }),
        ];
        for (case, claims) in cases {
            let refused = sealed_page_target(&event, &claims).unwrap_err();
            assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{case}");
        }
    }

    #[test]
    fn sealed_claims_of_a_removed_variant_fail_closed() {
        let mut event = page_event();
        event.variants = vec![page_variant("canary", "board-canary", Some((2, 0, 0)))];
        assert!(sealed_page_target(&event, &variant_claims()).is_ok());

        // The variant is deleted while a viewer still holds its sealed session.
        event.variants.clear();
        let refused = sealed_page_target(&event, &variant_claims()).unwrap_err();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);

        // A Shadow variant is never a page target either.
        let mut shadow = page_variant("canary", "board-canary", Some((2, 0, 0)));
        shadow.mode = EventVariantMode::Shadow { sample_rate: 1.0 };
        event.variants = vec![shadow];
        let refused = sealed_page_target(&event, &variant_claims()).unwrap_err();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn floating_variant_targets_accept_only_an_etag_bound_claim() {
        let mut event = page_event();
        event.variants = vec![page_variant("canary", "board-canary", None)];

        let mut claims = variant_claims();
        claims.target_board_version = None;
        claims.target_board_etag = Some("etag-canary".into());
        let canary = sealed_page_target(&event, &claims).unwrap();
        assert_eq!(canary.variant_name.as_deref(), Some("canary"));
        assert_eq!(canary.board_version, None);

        let refused = sealed_page_target(&event, &variant_claims()).unwrap_err();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn compiled_triggers_resolve_the_pinned_variant_and_refuse_unknown_pins() {
        let mut event = page_event();
        event.variants = vec![page_variant("canary", "board-canary", Some((2, 0, 0)))];

        assert!(pinned_page_target(&event, None).unwrap().is_none());
        let stable = pinned_page_target(&event, Some("stable")).unwrap().unwrap();
        assert_eq!(stable.variant_name, None);
        let canary = pinned_page_target(&event, Some("canary")).unwrap().unwrap();
        assert_eq!(canary.variant_name.as_deref(), Some("canary"));
        assert_eq!(canary.default_page_id.as_deref(), Some("page-canary"));

        let applied = variant::apply_target(event.clone(), &canary);
        assert_eq!(applied.board_id, "board-canary");
        assert_eq!(applied.board_version, Some((2, 0, 0)));
        assert_eq!(applied.default_page_id.as_deref(), Some("page-canary"));

        let unknown = pinned_page_target(&event, Some("nope")).unwrap_err();
        assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
        event.variants.clear();
        let removed = pinned_page_target(&event, Some("canary")).unwrap_err();
        assert_eq!(removed.status(), StatusCode::BAD_REQUEST);
    }

    fn contract_with_revision(target: &ResolvedTarget, revision: &str) -> ResolvedPageContract {
        let board = Board::new_detached(
            Some(target.board_id.clone()),
            flow_like_storage::Path::from("apps").child("app-1"),
        );
        let page_id = target.default_page_id.as_deref().unwrap();
        let page = flow_like::a2ui::Page::new(page_id, "Page", "/");
        let (page_execution, prerun, _) = compile_page_contract(&board, &page).unwrap();
        ResolvedPageContract {
            board_etag: None,
            page,
            page_execution,
            manifest_revision: revision.to_string(),
            prerun,
            entry_node_ids: HashSet::new(),
            entry_authority_revision: None,
        }
    }

    /// Stand-in for `resolve_page_contract`: each board compiles to its own
    /// revision, `board-broken` does not compile at all.
    fn revision_resolver(
        compiled: &std::cell::Cell<usize>,
    ) -> impl FnMut(ResolvedTarget) -> std::future::Ready<Result<ResolvedPageContract, ApiError>> + '_
    {
        move |target| {
            compiled.set(compiled.get() + 1);
            std::future::ready(match target.board_id.as_str() {
                "board-1" => Ok(contract_with_revision(&target, "rev-primary")),
                "board-canary" => Ok(contract_with_revision(&target, "rev-canary")),
                "board-broken" => Err(ApiError::bad_request(
                    "The Page Event configuration is invalid",
                )),
                other => panic!("unexpected board '{other}'"),
            })
        }
    }

    #[tokio::test]
    async fn unpinned_compiled_triggers_infer_the_variant_from_its_manifest_revision() {
        let mut event = page_event();
        event.variants = vec![page_variant("canary", "board-canary", Some((2, 0, 0)))];
        let compiled = std::cell::Cell::new(0);

        // The revision a canary bootstrap reported selects the variant.
        let (target, contract) =
            infer_compiled_target(&event, Some("rev-canary"), revision_resolver(&compiled))
                .await
                .unwrap();
        assert_eq!(target.variant_name.as_deref(), Some("canary"));
        assert_eq!(target.board_id, "board-canary");
        assert_eq!(target.default_page_id.as_deref(), Some("page-canary"));
        assert_eq!(contract.manifest_revision, "rev-canary");
        assert_eq!(compiled.get(), 2);

        // The primary's own revision never compiles a variant.
        compiled.set(0);
        let (target, contract) =
            infer_compiled_target(&event, Some("rev-primary"), revision_resolver(&compiled))
                .await
                .unwrap();
        assert_eq!(target.variant_name, None);
        assert_eq!(contract.manifest_revision, "rev-primary");
        assert_eq!(compiled.get(), 1);

        // A foreign revision keeps today's primary behavior.
        let (target, contract) =
            infer_compiled_target(&event, Some("rev-elsewhere"), revision_resolver(&compiled))
                .await
                .unwrap();
        assert_eq!(target.variant_name, None);
        assert_eq!(target.board_id, "board-1");
        assert_eq!(contract.manifest_revision, "rev-primary");

        // So does a missing revision; `require_compiled_revision` refuses it.
        compiled.set(0);
        let (target, _) = infer_compiled_target(&event, Some("  "), revision_resolver(&compiled))
            .await
            .unwrap();
        assert_eq!(target.variant_name, None);
        assert_eq!(compiled.get(), 1);
    }

    #[tokio::test]
    async fn revision_inference_needs_a_unique_compilable_variant_match() {
        let mut event = page_event();
        let compiled = std::cell::Cell::new(0);

        // Two variants compiling to the same revision are not a unique match.
        event.variants = vec![
            page_variant("canary", "board-canary", Some((2, 0, 0))),
            page_variant("twin", "board-canary", Some((2, 0, 0))),
        ];
        let (target, contract) =
            infer_compiled_target(&event, Some("rev-canary"), revision_resolver(&compiled))
                .await
                .unwrap();
        assert_eq!(target.variant_name, None);
        assert_eq!(contract.manifest_revision, "rev-primary");

        // A variant whose contract does not compile is skipped, not fatal.
        event.variants = vec![
            page_variant("broken", "board-broken", Some((2, 0, 0))),
            page_variant("canary", "board-canary", Some((2, 0, 0))),
        ];
        let (target, _) =
            infer_compiled_target(&event, Some("rev-canary"), revision_resolver(&compiled))
                .await
                .unwrap();
        assert_eq!(target.variant_name.as_deref(), Some("canary"));

        // The primary failing to compile is still the request's error.
        event.board_id = "board-broken".to_string();
        let refused =
            infer_compiled_target(&event, Some("rev-canary"), revision_resolver(&compiled))
                .await
                .err()
                .expect("a primary that does not compile fails the request");
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
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
    fn compiled_targets_tolerate_a_superseded_revision_but_not_a_missing_one() {
        // Every unrelated Board edit supersedes the revision a rendered Page
        // holds; a compiled target must survive that and be resolved against
        // the current contract instead.
        assert!(require_compiled_revision(Some("old"), "current").is_ok());
        assert!(require_compiled_revision(Some("current"), "current").is_ok());
        // Having gone through a bootstrap at all is still required.
        assert!(require_compiled_revision(None, "current").is_err());
        assert!(require_compiled_revision(Some(""), "current").is_err());
    }
}
