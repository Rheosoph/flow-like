use std::collections::{HashMap, HashSet};

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::event::{Event, ReleaseNotes};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};

use super::db::{filter_event_secrets, get_event_from_db_opt, redact_page_event_board_metadata};
use super::get_event::map_missing_event_artifact;

/// Bound on how many archived versions one response projects, matching the
/// version-archive retention default. Listings past it set `truncated`.
const TIMELINE_VERSION_CAP: usize = 200;

/// One event revision — the live head or an archived version snapshot.
#[derive(Serialize, ToSchema)]
pub struct EventTimelineEntry {
    /// Event version as `[major, minor, patch]`. The live head carries the
    /// current `event_version`.
    #[schema(value_type = Vec<u32>)]
    pub version: (u32, u32, u32),
    /// Dotted `MAJOR.MINOR.PATCH` — the same format the Lance `runs` table
    /// stores in `event_version`, so runs group against entries by this key.
    pub version_key: String,
    pub is_live: bool,
    pub name: String,
    pub description: String,
    pub event_type: String,
    pub active: bool,
    pub board_id: Option<String>,
    /// Pinned board version as `[major, minor, patch]`; absent when the
    /// revision floats on the board's latest version.
    #[schema(value_type = Option<Vec<u32>>)]
    pub board_version: Option<(u32, u32, u32)>,
    pub node_id: Option<String>,
    pub default_page_id: Option<String>,
    pub route: Option<String>,
    pub is_default: bool,
    pub execution_mode: String,
    pub exposure: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    /// Whether the revision's target board still loads.
    pub board_resolves: bool,
    /// Whether the revision's target node still exists on that board.
    pub node_resolves: bool,
    /// Variable IDs only — values are never part of the timeline.
    pub variable_ids: Vec<String>,
    /// IDs of secret variables — values are never part of the timeline.
    pub secret_variable_ids: Vec<String>,
    /// Release-notes kind attached to the revision: "notes" or "url".
    pub notes_kind: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct EventTimelineResponse {
    pub event_id: String,
    /// Distinct board IDs across all entries, the live head's board first.
    pub boards: Vec<String>,
    /// The archive listing hit the version cap; older entries are not shown.
    pub truncated: bool,
    /// Archived versions that were listed but could not be loaded.
    pub skipped: u32,
    /// Live head first, then archived versions newest-first.
    pub entries: Vec<EventTimelineEntry>,
}

fn system_time_ms(time: std::time::SystemTime) -> u64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

type ResolutionCache = HashMap<(String, Option<(u32, u32, u32)>), Option<HashSet<String>>>;

/// Preflight the revision's `(board_id, board_version, node_id)` target. Any
/// load error is `(false, false)` — a missing board must mark the entry, never
/// fail the listing.
async fn target_resolution(
    state: &AppState,
    app_id: &str,
    cache: &mut ResolutionCache,
    event: &Event,
) -> (bool, bool) {
    if event.board_id.is_empty() {
        return (false, false);
    }

    let key = (event.board_id.clone(), event.board_version);
    if !cache.contains_key(&key) {
        let nodes = match state
            .master_board_shared(app_id, &event.board_id, state, event.board_version)
            .await
        {
            Ok(cached) => Some(cached.board.nodes.keys().cloned().collect::<HashSet<_>>()),
            Err(error) => {
                tracing::debug!(
                    app_id = %app_id,
                    board_id = %event.board_id,
                    board_version = ?event.board_version,
                    %error,
                    "Timeline target board does not resolve"
                );
                None
            }
        };
        cache.insert(key.clone(), nodes);
    }

    match cache.get(&key) {
        Some(Some(nodes)) => {
            // Page events carry no node target — the entry resolves iff its
            // board does.
            let node_resolves = event.node_id.is_empty() || nodes.contains(&event.node_id);
            (true, node_resolves)
        }
        _ => (false, false),
    }
}

fn project_entry(
    event: Event,
    is_live: bool,
    board_resolves: bool,
    node_resolves: bool,
    redact_board_metadata: bool,
) -> EventTimelineEntry {
    let event = filter_event_secrets(event);
    let event = if redact_board_metadata {
        redact_page_event_board_metadata(event)
    } else {
        event
    };

    let mut variable_ids: Vec<String> = event.variables.keys().cloned().collect();
    variable_ids.sort_unstable();
    let mut secret_variable_ids: Vec<String> = event
        .variables
        .iter()
        .filter(|(_, variable)| variable.secret)
        .map(|(id, _)| id.clone())
        .collect();
    secret_variable_ids.sort_unstable();

    EventTimelineEntry {
        version: event.event_version,
        version_key: super::dotted_version_key(event.event_version),
        is_live,
        board_id: (!event.board_id.is_empty()).then(|| event.board_id.clone()),
        board_version: event.board_version,
        node_id: (!event.node_id.is_empty()).then(|| event.node_id.clone()),
        name: event.name,
        description: event.description,
        event_type: event.event_type,
        active: event.active,
        default_page_id: event.default_page_id,
        route: event.route,
        is_default: event.is_default,
        execution_mode: event.execution_mode.as_str().to_string(),
        exposure: event.exposure.as_str().to_string(),
        created_at_ms: system_time_ms(event.created_at),
        updated_at_ms: system_time_ms(event.updated_at),
        board_resolves,
        node_resolves,
        variable_ids,
        secret_variable_ids,
        notes_kind: event.notes.as_ref().map(|notes| {
            match notes {
                ReleaseNotes::NOTES(_) => "notes",
                ReleaseNotes::URL(_) => "url",
            }
            .to_string()
        }),
    }
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/timeline",
    tag = "events",
    description = "Version history for an event: the live configuration followed by every archived version, with target health and run-grouping keys.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Event version timeline", body = EventTimelineResponse),
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
    name = "GET /apps/{app_id}/events/{event_id}/timeline",
    skip(state, user)
)]
pub async fn get_event_timeline(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
) -> Result<Json<EventTimelineResponse>, ApiError> {
    // ReadEvents with no ExecuteEvents fallback: the timeline exposes notes
    // and variable IDs that the execute-only projection deliberately strips.
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);
    let sub = permission.sub()?;

    // Verbatim mirror of GET /events/{event_id}'s redaction gate. ReadEvents
    // being mandatory here makes the redaction branch inert today; it is kept
    // so both surfaces stay behaviorally identical if this gate ever widens.
    let has_read = permission.has_permission(RolePermissions::ReadEvents);
    let can_use_direct_board = permission.has_permission(RolePermissions::ReadBoards)
        && permission.has_permission(RolePermissions::ExecuteBoards);
    let redact_board_metadata = !has_read && !can_use_direct_board;

    let app = state.master_app(&sub, &app_id, &state).await?;

    // The live head is synthesized from the database row — the archive is
    // written at the pre-bump version, so the live version never appears in
    // `versions/`. Older apps can miss the row; repair-free bucket fallback.
    let live = if let Some(event) = get_event_from_db_opt(&state.db, &event_id, &app_id).await? {
        event
    } else {
        let event = app
            .get_event(&event_id, None)
            .await
            .map_err(|error| map_missing_event_artifact(&event_id, error))?;
        if event.id != event_id {
            tracing::error!(
                expected_event_id = %event_id,
                artifact_event_id = %event.id,
                app_id = %app_id,
                "Event artifact ID does not match the requested event"
            );
            return Err(ApiError::internal("Event artifact ID mismatch"));
        }
        event
    };

    let versions = live.get_versions(&app).await?;
    let truncated = versions.len() > TIMELINE_VERSION_CAP;
    let live_version = live.event_version;

    let mut resolution_cache: ResolutionCache = HashMap::new();
    let mut skipped: u32 = 0;
    let mut entries = Vec::with_capacity(versions.len().min(TIMELINE_VERSION_CAP) + 1);

    let (board_resolves, node_resolves) =
        target_resolution(&state, &app_id, &mut resolution_cache, &live).await;
    entries.push(project_entry(
        live,
        true,
        board_resolves,
        node_resolves,
        redact_board_metadata,
    ));

    for version in versions.into_iter().take(TIMELINE_VERSION_CAP) {
        // The archive holds the live version only after a crash between the
        // archive and live writes — identical content, so keep the head alone.
        if version == live_version {
            continue;
        }
        let event = match app.get_event(&event_id, Some(version)).await {
            Ok(event) if event.id == event_id => event,
            Ok(event) => {
                tracing::warn!(
                    expected_event_id = %event_id,
                    artifact_event_id = %event.id,
                    app_id = %app_id,
                    version = ?version,
                    "Archived event version carries a foreign event ID; skipping"
                );
                skipped += 1;
                continue;
            }
            Err(error) => {
                tracing::warn!(
                    event_id = %event_id,
                    app_id = %app_id,
                    version = ?version,
                    %error,
                    "Failed to load archived event version; skipping"
                );
                skipped += 1;
                continue;
            }
        };

        let (board_resolves, node_resolves) =
            target_resolution(&state, &app_id, &mut resolution_cache, &event).await;
        entries.push(project_entry(
            event,
            false,
            board_resolves,
            node_resolves,
            redact_board_metadata,
        ));
    }

    let mut boards: Vec<String> = Vec::new();
    for entry in &entries {
        if let Some(board_id) = &entry.board_id
            && !boards.iter().any(|known| known == board_id)
        {
            boards.push(board_id.clone());
        }
    }

    Ok(Json(EventTimelineResponse {
        event_id,
        boards,
        truncated,
        skipped,
        entries,
    }))
}
