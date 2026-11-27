//! Pre-run analysis endpoint for events
//!
//! Returns information needed before executing an event. Frontends are
//! expected to cache responses and revalidate in the background using the
//! `signature` field to detect drift.

use crate::{
    ensure_fresh_permission, ensure_permission,
    error::ApiError,
    execution::variant,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::prerun_shared::{
        OAuthRequirement, PrerunPayload, RuntimeVariable, load_prerun_manifest, parse_version,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use flow_like::flow::{board::ExecutionMode, event::EventExecutionMode, node::NodePermission};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

use super::db::get_event_from_db;
use super::page_trigger::PageTrigger;

/// Query parameters for pre-run analysis
#[derive(Debug, Deserialize, ToSchema)]
pub struct PrerunEventQuery {
    /// Board version as tuple (major, minor, patch) - defaults to latest
    pub version: Option<String>,
    /// Page prerun only: the served variant (`stable` or a live variant name)
    /// the bootstrap reported, so the contract is resolved for that target.
    #[serde(default, rename = "__variant")]
    pub variant: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PrerunPageEventRequest {
    /// Governed Page selector. A capability JWT, when present, remains body
    /// data and does not replace normal request authentication.
    pub page_trigger: PageTrigger,
}

/// Response from pre-run analysis
#[derive(Debug, Serialize, ToSchema)]
pub struct PrerunEventResponse {
    /// ID of the board this event triggers
    pub board_id: String,
    /// Variables that are marked as runtime_configured (need user-provided values)
    pub runtime_variables: Vec<RuntimeVariable>,
    /// OAuth providers required by nodes in this board
    pub oauth_requirements: Vec<OAuthRequirement>,
    /// Whether the event can only run locally (has offline-only nodes)
    pub requires_local_execution: bool,
    /// Board's execution mode setting (Hybrid, Remote, Local)
    #[schema(value_type = String)]
    pub execution_mode: ExecutionMode,
    /// Event's execution mode — where this specific event runs.
    /// An event is always either Local or Remote (never Hybrid), and must
    /// match the board when the board is not Hybrid.
    #[schema(value_type = String)]
    pub event_execution_mode: EventExecutionMode,
    /// Whether the caller satisfies this route's local permission policy and
    /// the Event is not pinned to Remote. If false, execution stays server-side.
    pub can_execute_locally: bool,
    /// Whether the board contains any WASM (external) nodes
    pub has_wasm_nodes: bool,
    /// package_id values of all WASM nodes present in the board
    pub wasm_package_ids: Vec<String>,
    /// Per-package permissions declared by WASM nodes (package_id -> list of permissions).
    /// Ordered, so a bootstrap that embeds this answer keeps a stable ETag.
    pub wasm_package_permissions: BTreeMap<String, Vec<NodePermission>>,
    /// Stable hash over the board-derived fields. Frontends may cache the
    /// response and revalidate in the background; a changed signature
    /// signals the underlying board has shifted.
    pub signature: String,
    /// Set for a governed Page prerun.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    /// Authority revision attached to static actions and lifecycle calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_revision: Option<String>,
}

pub(crate) fn build_response(
    board_id: String,
    payload: PrerunPayload,
    event_execution_mode: EventExecutionMode,
    can_execute_locally: bool,
    page_id: Option<String>,
    manifest_revision: Option<String>,
) -> PrerunEventResponse {
    PrerunEventResponse {
        board_id,
        runtime_variables: payload.runtime_variables,
        oauth_requirements: payload.oauth_requirements,
        requires_local_execution: payload.requires_local_execution,
        execution_mode: payload.execution_mode,
        event_execution_mode,
        can_execute_locally,
        has_wasm_nodes: payload.has_wasm_nodes,
        wasm_package_ids: payload.wasm_package_ids,
        wasm_package_permissions: payload.wasm_package_permissions.into_iter().collect(),
        signature: payload.signature,
        page_id,
        manifest_revision,
    }
}

fn page_board_id_for_response(board_id: String, can_execute_locally: bool) -> String {
    can_execute_locally.then_some(board_id).unwrap_or_default()
}

/// What the page prerun route answers for a lifecycle (`Special`) trigger of a resolved Page
/// contract. `can_use_direct_board` is the caller's ReadBoards and ExecuteBoards.
pub(crate) fn page_prerun_response(
    board_id: String,
    prerun: PrerunPayload,
    event_execution_mode: EventExecutionMode,
    can_use_direct_board: bool,
    page_id: String,
    manifest_revision: String,
) -> PrerunEventResponse {
    let can_execute_locally =
        can_use_direct_board && event_execution_mode != EventExecutionMode::Remote;
    build_response(
        page_board_id_for_response(board_id, can_execute_locally),
        prerun,
        event_execution_mode,
        can_execute_locally,
        Some(page_id),
        Some(manifest_revision),
    )
}

/// Analyze an event to determine what's needed before execution.
///
/// Returns runtime-configured variables and OAuth requirements from the event's board.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/prerun",
    tag = "events",
    description = "Get pre-run requirements for an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("version" = Option<String>, Query, description = "Version in MAJOR_MINOR_PATCH format")
    ),
    responses(
        (status = 200, description = "Pre-run requirements", body = PrerunEventResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/prerun",
    skip(state, user, query)
)]
pub async fn prerun_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<PrerunEventQuery>,
) -> Result<Json<PrerunEventResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    // Loading the board used to require a caller identity; principals without
    // one (API keys, connected apps) stay rejected until that is decided on
    // purpose.
    permission.sub()?;

    let version = query.version.as_ref().and_then(|v| parse_version(v));

    let event = get_event_from_db(&state.db, &event_id, &app_id).await?;
    super::ensure_source_execution_allowed(&event)?;
    // Prerun discloses the full board/flow definition; hold connected apps to
    // the same surface policy as invoke (only directly-callable events, never
    // the REST/MCP events they must reach through the proxy).
    super::ensure_connected_app_direct_event_allowed(&user, &event.event_type, event.active)?;
    if event.default_page_id.is_some() {
        return Err(ApiError::bad_request(
            "Page Event prerun requires POST with page_trigger",
        ));
    }
    let board_id = event.board_id.clone();
    let event_execution_mode = event.execution_mode;
    // A Remote event never runs on the caller's device, and its board is not
    // expected to be there. Saying otherwise sends clients down a local path
    // that can only end in a missing board.
    let can_execute_locally = permission.has_permission(RolePermissions::ReadBoards)
        && event_execution_mode != EventExecutionMode::Remote;

    let manifest = load_prerun_manifest(&state, &app_id, &board_id, version).await?;

    Ok(Json(build_response(
        board_id,
        PrerunPayload::from(&*manifest),
        event_execution_mode,
        can_execute_locally,
        None,
        None,
    )))
}

/// Resolve prerun requirements for one governed Page trigger. Unlike the GET
/// route, this path cannot be used to choose an arbitrary board or version.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/events/{event_id}/prerun",
    tag = "events",
    description = "Resolve pre-run requirements for one governed Page trigger.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("version" = Option<String>, Query, description = "Configured Page board version in MAJOR_MINOR_PATCH format"),
        ("__variant" = Option<String>, Query, description = "The served variant reported by bootstrap (`stable` or a live variant name)"),
        ("x-flow-like-variant" = Option<String>, Header, description = "The served variant reported by bootstrap (`stable` or a live variant name)")
    ),
    request_body = PrerunPageEventRequest,
    responses(
        (status = 200, description = "Pre-run requirements and resolved Page authority revision", body = PrerunEventResponse),
        (status = 400, description = "Invalid or stale Page trigger"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/events/{event_id}/prerun",
    skip(state, user, query, headers, body)
)]
pub async fn prerun_page_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<PrerunEventQuery>,
    headers: HeaderMap,
    Json(body): Json<PrerunPageEventRequest>,
) -> Result<Json<PrerunEventResponse>, ApiError> {
    let permission =
        ensure_fresh_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let event = get_event_from_db(&state.db, &event_id, &app_id).await?;
    super::ensure_source_execution_allowed(&event)?;
    super::ensure_connected_app_direct_event_allowed(&user, &event.event_type, event.active)?;

    let requested_version = query
        .version
        .as_deref()
        .map(|requested| {
            super::parse_version_tuple(requested)
                .ok_or_else(|| ApiError::bad_request("version must use MAJOR_MINOR_PATCH format"))
        })
        .transpose()?;

    let variant_pin = variant::pin_from_request(&headers, query.variant.clone());
    let resolved = super::page_trigger::resolve_page_trigger(
        &state,
        &permission,
        &app_id,
        &event,
        &body.page_trigger,
        variant_pin.as_deref(),
    )
    .await?;
    // The served target (primary or bootstrap-assigned variant) owns the
    // version; a request may only restate it.
    if requested_version.is_some() && resolved.board_version != requested_version {
        return Err(ApiError::bad_request(
            "A Page prerun always uses the Event's configured board version",
        ));
    }
    let can_execute_locally = permission.has_permission(RolePermissions::ReadBoards)
        && permission.has_permission(RolePermissions::ExecuteBoards)
        && event.execution_mode != EventExecutionMode::Remote
        && matches!(
            body.page_trigger,
            PageTrigger::Action {
                capability_jwt: None,
                ..
            } | PageTrigger::Special { .. }
        );

    let board_id = page_board_id_for_response(resolved.board_id, can_execute_locally);
    Ok(Json(build_response(
        board_id,
        resolved.prerun,
        event.execution_mode,
        can_execute_locally,
        Some(resolved.page_id),
        Some(resolved.manifest_revision),
    )))
}

#[cfg(test)]
mod tests {
    use super::{
        EventExecutionMode, ExecutionMode, PrerunPayload, page_board_id_for_response,
        page_prerun_response,
    };
    use std::collections::HashMap;

    fn payload() -> PrerunPayload {
        PrerunPayload {
            runtime_variables: Vec::new(),
            oauth_requirements: Vec::new(),
            requires_local_execution: false,
            execution_mode: ExecutionMode::Hybrid,
            has_wasm_nodes: false,
            wasm_package_ids: Vec::new(),
            wasm_package_permissions: HashMap::new(),
            signature: "sig".into(),
        }
    }

    #[test]
    fn a_page_lifecycle_prerun_names_the_board_only_to_callers_who_may_run_it_locally() {
        let local = page_prerun_response(
            "board-1".into(),
            payload(),
            EventExecutionMode::Local,
            true,
            "page-1".into(),
            "per2_rev".into(),
        );
        assert!(local.can_execute_locally);
        assert_eq!(local.board_id, "board-1");
        assert_eq!(local.page_id.as_deref(), Some("page-1"));
        assert_eq!(local.manifest_revision.as_deref(), Some("per2_rev"));

        let pinned_remote = page_prerun_response(
            "board-1".into(),
            payload(),
            EventExecutionMode::Remote,
            true,
            "page-1".into(),
            "per2_rev".into(),
        );
        assert!(!pinned_remote.can_execute_locally);
        assert_eq!(pinned_remote.board_id, "");

        let viewer = page_prerun_response(
            "board-1".into(),
            payload(),
            EventExecutionMode::Local,
            false,
            "page-1".into(),
            "per2_rev".into(),
        );
        assert!(!viewer.can_execute_locally);
        assert_eq!(viewer.board_id, "");
    }

    #[test]
    fn runtime_only_page_prerun_omits_board_identity() {
        assert_eq!(page_board_id_for_response("board-1".into(), false), "");
        assert_eq!(
            page_board_id_for_response("board-1".into(), true),
            "board-1"
        );
    }
}
