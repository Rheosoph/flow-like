use flow_like::{
    app::App,
    flow::{
        board::VersionType,
        event::{
            Event, ReleaseNotes, RestoreIssueSeverity, RestoreOptions, RestorePlan,
            filter_event_secrets,
        },
        execution::{LogMeta, run_index::RunQuery},
        oauth::OAuthToken,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;

use crate::{
    event_sink::EventSinkManager, functions::TauriFunctionError, state::TauriFlowLikeState,
};

/// Bound on how many archived versions one response projects, matching the
/// API endpoint. Listings past it set `truncated`.
const TIMELINE_VERSION_CAP: usize = 200;
const DEFAULT_RUNS_LIMIT: u64 = 100;
/// Hard cap on one page of merged run summaries, matching the API endpoint.
const MAX_RUNS_LIMIT: u64 = 200;

/// Wire twin of the API's `GET /apps/{app_id}/events/{event_id}/timeline`
/// response. Field names and serde casing must stay identical so the UI
/// renders both transports unchanged.
#[derive(Serialize, Debug, Clone)]
pub struct EventTimelineResponse {
    pub event_id: String,
    /// Distinct board ids across all entries, the live head's board first.
    pub boards: Vec<String>,
    /// The archive listing hit the version cap; older entries are not shown.
    pub truncated: bool,
    /// Archived versions that were listed but could not be loaded.
    pub skipped: u32,
    /// Live head first, then archived versions newest-first.
    pub entries: Vec<EventTimelineEntry>,
}

#[derive(Serialize, Debug, Clone)]
pub struct EventTimelineEntry {
    pub version: (u32, u32, u32),
    /// Dotted `MAJOR.MINOR.PATCH` — the same format the run index stores in
    /// `event_version`, so runs group against entries by this key.
    pub version_key: String,
    pub is_live: bool,
    pub name: String,
    pub description: String,
    pub event_type: String,
    pub active: bool,
    pub board_id: Option<String>,
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
    /// Ids only, never values — secrets are filtered before projection.
    pub variable_ids: Vec<String>,
    pub secret_variable_ids: Vec<String>,
    /// "notes" | "url"
    pub notes_kind: Option<String>,
}

/// Wire twin of the API's `GET /apps/{app_id}/events/{event_id}/runs`
/// response.
#[derive(Serialize, Debug, Clone)]
pub struct EventRunsResponse {
    pub runs: Vec<LogMeta>,
    /// Every validated board the run index was queried for.
    pub boards_queried: Vec<String>,
}

/// Dotted `MAJOR.MINOR.PATCH` — the one event-version key format, shared by
/// timeline entries and the run index's `event_version` column. The board
/// `version` column in the same index uses `v{major}-{minor}-{patch}`; mixing
/// the two silently breaks version grouping.
fn dotted_version_key(version: (u32, u32, u32)) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

/// Ids name log-store paths and per-run tables, so only `create_id()`-shaped
/// values may pass; anything that could escape a path segment or a quoted
/// filter literal never reaches storage.
pub(crate) fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn system_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

type ResolutionCache = HashMap<(String, Option<(u32, u32, u32)>), Option<HashSet<String>>>;

/// Preflight the revision's `(board_id, board_version, node_id)` target. Any
/// load error is `(false, false)` — a missing board must mark the entry, never
/// fail the listing.
async fn target_resolution(app: &App, cache: &mut ResolutionCache, event: &Event) -> (bool, bool) {
    if event.board_id.is_empty() {
        return (false, false);
    }

    let key = (event.board_id.clone(), event.board_version);
    if !cache.contains_key(&key) {
        let nodes = match app
            .open_board(event.board_id.clone(), Some(false), event.board_version)
            .await
        {
            Ok(board) => Some(
                board
                    .snapshot()
                    .nodes
                    .keys()
                    .cloned()
                    .collect::<HashSet<_>>(),
            ),
            Err(error) => {
                tracing::debug!(
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

fn project_timeline_entry(
    event: Event,
    is_live: bool,
    board_resolves: bool,
    node_resolves: bool,
) -> EventTimelineEntry {
    // The desktop assembles from raw local artifacts, so secrets are filtered
    // here — before projection — exactly like the API endpoint. The local
    // caller is owner-equivalent, so the API's execute-only and page-metadata
    // redactions do not apply.
    let event = filter_event_secrets(event);

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
        version_key: dotted_version_key(event.event_version),
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

/// Local timeline over the event's version archive. The live head is the
/// device's live event object (the desktop has no database mirror) — the
/// archive is written at the pre-bump version, so the live version is never
/// present in `versions/`. Secret values are filtered from every entry before
/// projection.
#[tauri::command(async)]
pub async fn get_event_timeline(
    handler: AppHandle,
    app_id: String,
    event_id: String,
) -> Result<EventTimelineResponse, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id.clone(), flow_like_state)
        .await
        .map_err(|_| TauriFunctionError::new("App not found"))?;

    let live = app.get_event(&event_id, None).await?;
    let versions = live.get_versions(&app).await?;
    let truncated = versions.len() > TIMELINE_VERSION_CAP;
    let live_version = live.event_version;

    let mut cache: ResolutionCache = HashMap::new();
    let mut skipped: u32 = 0;
    let mut entries = Vec::with_capacity(versions.len().min(TIMELINE_VERSION_CAP) + 1);

    let (board_resolves, node_resolves) = target_resolution(&app, &mut cache, &live).await;
    entries.push(project_timeline_entry(
        live,
        true,
        board_resolves,
        node_resolves,
    ));

    for version in versions.into_iter().take(TIMELINE_VERSION_CAP) {
        // The archive holds the live version only after a crash between the
        // archive and live writes — identical content, so keep the head alone.
        if version == live_version {
            continue;
        }
        let event = match Event::load(&event_id, &app, Some(version)).await {
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

        let (board_resolves, node_resolves) = target_resolution(&app, &mut cache, &event).await;
        entries.push(project_timeline_entry(
            event,
            false,
            board_resolves,
            node_resolves,
        ));
    }

    let mut boards: Vec<String> = Vec::new();
    for entry in &entries {
        if let Some(board_id) = &entry.board_id
            && !boards.contains(board_id)
        {
            boards.push(board_id.clone());
        }
    }

    Ok(EventTimelineResponse {
        event_id,
        boards,
        truncated,
        skipped,
        entries,
    })
}

/// Run telemetry for an event across the boards its timeline touched, read
/// from the local run index. Mirrors the API endpoint's semantics: callers
/// feed `board_ids` from the timeline's `boards` (falling back to the live
/// event's board when absent), every id is allowlisted and validated against
/// the app, and results come back newest-first across boards.
#[tauri::command(async)]
pub async fn list_event_runs(
    handler: AppHandle,
    app_id: String,
    event_id: String,
    board_ids: Option<Vec<String>>,
    limit: Option<u64>,
    offset: Option<u64>,
) -> Result<EventRunsResponse, TauriFunctionError> {
    let limit = limit.unwrap_or(DEFAULT_RUNS_LIMIT).min(MAX_RUNS_LIMIT) as usize;
    let offset = offset.unwrap_or(0) as usize;

    if !is_safe_id(&event_id) {
        return Err(TauriFunctionError::new(
            "Event ID may only contain alphanumeric characters, '-' and '_'",
        ));
    }

    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id.clone(), flow_like_state.clone())
        .await
        .map_err(|_| TauriFunctionError::new("App not found"))?;

    let mut board_ids: Vec<String> = {
        let mut deduped = Vec::new();
        for board_id in board_ids.unwrap_or_default() {
            if !deduped.contains(&board_id) {
                deduped.push(board_id);
            }
        }
        deduped
    };

    if board_ids.is_empty() {
        // The timeline's board list is the intended source of the board set;
        // without it, fall back to the live event's current board.
        let live = app.get_event(&event_id, None).await?;
        if live.board_id.is_empty() {
            return Err(TauriFunctionError::new(
                "Event has no board target; pass at least one board id",
            ));
        }
        board_ids.push(live.board_id);
    }

    for board_id in &board_ids {
        if !is_safe_id(board_id) {
            return Err(TauriFunctionError::new(
                "Board IDs may only contain alphanumeric characters, '-' and '_'",
            ));
        }
        if !app.boards.contains(board_id) {
            return Err(TauriFunctionError::new(&format!(
                "Board {board_id} does not belong to this app"
            )));
        }
    }

    let index = crate::run_index::registered_run_index(&flow_like_state).await?;
    let runs = index
        .list(&RunQuery {
            app_id,
            board_ids: board_ids.clone(),
            event_id: Some(event_id),
            limit,
            offset,
            include_nodes: true,
            ..RunQuery::default()
        })
        .await?;

    Ok(EventRunsResponse {
        runs,
        boards_queried: board_ids,
    })
}

#[tauri::command(async)]
pub async fn get_event(
    handler: AppHandle,
    app_id: String,
    event_id: String,
    version: Option<(u32, u32, u32)>,
) -> Result<Event, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Ok(app) = App::load(app_id.clone(), flow_like_state).await {
        let event = app.get_event(&event_id, version).await?;
        return Ok(event);
    }

    Err(TauriFunctionError::new("Event not found"))
}

#[tauri::command(async)]
pub async fn get_event_versions(
    handler: AppHandle,
    app_id: String,
    event_id: String,
) -> Result<Vec<(u32, u32, u32)>, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Ok(app) = App::load(app_id, flow_like_state).await {
        let versions = app.get_event_versions(&event_id).await?;
        return Ok(versions);
    }

    Err(TauriFunctionError::new("Event not found"))
}

#[tauri::command(async)]
pub async fn get_events(
    handler: AppHandle,
    app_id: String,
) -> Result<Vec<Event>, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Ok(app) = App::load(app_id.clone(), flow_like_state).await {
        let events = &app.events;
        let mut loaded_events = Vec::with_capacity(events.len());

        for event in events {
            if let Ok(loaded_event) = Event::load(event, &app, None).await {
                if loaded_event.event_type != "ontology_action" {
                    loaded_events.push(loaded_event);
                }
            } else {
                tracing::warn!("Failed to load event: {} in app {}", event, app_id.clone());
            }
        }

        return Ok(loaded_events);
    }

    Err(TauriFunctionError::new("Events not found"))
}

/// How a save treats the event's trigger on this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkRegistration {
    /// The user approved the trigger: register it, or refresh an existing one.
    Register,
    /// The user declined the trigger: remove it, keeping its credentials.
    Skip,
    /// Refresh an existing registration only, never create one. The default,
    /// so cache mirrors and other unattended saves cannot start a trigger.
    Keep,
}

/// Applies `mode` for the saved `event`. `pat` and `oauth_tokens` merge into
/// the registration's stored credentials.
async fn sync_local_sink(
    handler: &AppHandle,
    app_id: &str,
    event: &Event,
    offline: Option<bool>,
    pat: Option<String>,
    oauth_tokens: Option<HashMap<String, OAuthToken>>,
    mode: SinkRegistration,
) -> anyhow::Result<()> {
    let manager_state = crate::state::TauriEventSinkManagerState::construct(handler).await?;
    let manager = manager_state.lock().await;
    match mode {
        SinkRegistration::Register => {
            manager
                .register_from_flow_event(handler, app_id, event, offline, pat, oauth_tokens)
                .await
        }
        SinkRegistration::Keep => {
            if manager.get_registration(&event.id)?.is_none() {
                return Ok(());
            }
            manager
                .register_from_flow_event(handler, app_id, event, offline, pat, oauth_tokens)
                .await
        }
        SinkRegistration::Skip => manager.decline_local_registration(handler, &event.id).await,
    }
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub async fn upsert_event(
    handler: AppHandle,
    app_id: String,
    event: Event,
    version_type: Option<VersionType>,
    enforce_id: Option<bool>,
    offline: Option<bool>,
    pat: Option<String>,
    oauth_tokens: Option<HashMap<String, OAuthToken>>,
    register_sink: Option<SinkRegistration>,
) -> Result<Event, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Ok(mut app) = App::load(app_id.clone(), flow_like_state).await {
        if event.event_type == "ontology_action"
            || app
                .get_event(&event.id, None)
                .await
                .is_ok_and(|saved| saved.event_type == "ontology_action")
        {
            return Err(TauriFunctionError::new(
                "Ontology action events are managed through Data Studio actions",
            ));
        }
        let event = app.upsert_event(event, version_type, enforce_id).await?;

        let mode = register_sink.unwrap_or(SinkRegistration::Keep);
        if let Err(e) =
            sync_local_sink(&handler, &app_id, &event, offline, pat, oauth_tokens, mode).await
        {
            println!(
                "Failed to update sink registration of event {}: {}",
                event.id, e
            );
            tracing::warn!(
                "Failed to update sink registration of event {}: {}",
                event.id,
                e
            );
            if event.event_type == "geolocation" {
                return Err(TauriFunctionError::new(&format!(
                    "Location Event was saved, but device monitoring could not be updated: {e}"
                )));
            }
        }

        return Ok(event);
    }

    Err(TauriFunctionError::new("Failed to upsert event"))
}

/// What saving `event` does to this device's triggers, so the UI can ask
/// before a new one starts running here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalSinkPlan {
    /// Nothing registers locally: unsupported type, inactive, Remote, or a
    /// remote-only sink.
    None,
    /// A registration of the same trigger type exists; saving refreshes it.
    Existing,
    /// A trigger would start on this device, for the first time or with a
    /// new type.
    New,
}

#[tauri::command(async)]
pub async fn local_sink_registration_plan(
    handler: AppHandle,
    app_id: String,
    mut event: Event,
) -> Result<LocalSinkPlan, TauriFunctionError> {
    // Saving reconciles the execution mode with the board, so a stale client
    // copy must not decide whether the user is asked.
    if let Ok(state) = TauriFlowLikeState::construct(&handler).await
        && let Ok(app) = App::load(app_id, state).await
    {
        let _ = event.reconcile_execution_mode_with_board(&app).await;
    }
    if EventSinkManager::local_registration_blocker(&event).is_some() {
        return Ok(LocalSinkPlan::None);
    }

    let manager_state = crate::state::TauriEventSinkManagerState::construct(&handler).await?;
    let existing = manager_state.lock().await.get_registration(&event.id)?;
    Ok(match existing {
        Some(registration) if registration.r#type == event.event_type => LocalSinkPlan::Existing,
        _ => LocalSinkPlan::New,
    })
}

/// Wire twin of the API's `POST /apps/{app_id}/events/{event_id}/restore`
/// response. `plan.restored` (and `event`) are serialized with secret variable
/// values blanked — the plan never carries a secret.
#[derive(Serialize, Debug, Clone)]
pub struct RestoreEventResponse {
    pub plan: RestorePlan,
    /// The event as persisted — present only after a non-dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Event>,
    /// Outcome of the non-fatal sink re-registration after a non-dry run. A
    /// failure here does not roll the restore back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_status: Option<String>,
}

/// Plan or apply a forward-only restore of an archived event version against
/// this device's live event and archive. Version addressing is local-only by
/// construction: the local and cloud version counters diverge once edits land
/// on both sides, so a tuple from a cloud timeline must never be restored
/// here. On synced apps the local secrets are already server-filtered, so
/// `SecretUnrecoverable` fires often — it blocks a non-dry run unless
/// `accept_blank_secrets` downgrades it, exactly like core.
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub async fn restore_event(
    handler: AppHandle,
    app_id: String,
    event_id: String,
    version: (u32, u32, u32),
    version_type: Option<VersionType>,
    dry_run: Option<bool>,
    restore_route: Option<bool>,
    drop_canary: Option<bool>,
    accept_blank_secrets: Option<bool>,
    offline: Option<bool>,
    register_sink: Option<SinkRegistration>,
) -> Result<RestoreEventResponse, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let mut app = App::load(app_id.clone(), flow_like_state)
        .await
        .map_err(|_| TauriFunctionError::new("App not found"))?;

    let live = app.get_event(&event_id, None).await?;
    if live.event_type == "ontology_action" {
        return Err(TauriFunctionError::new(
            "Ontology action events are managed through Data Studio actions",
        ));
    }

    let options = RestoreOptions {
        restore_route: restore_route.unwrap_or(false),
        drop_canary: drop_canary.unwrap_or(false),
        accept_blank_secrets: accept_blank_secrets.unwrap_or(false),
    };
    let mut plan = Event::plan_restore(&app, &event_id, version, &live, &options).await?;
    if plan.restored.event_type == "ontology_action" {
        return Err(TauriFunctionError::new(
            "Ontology action events are managed through Data Studio actions",
        ));
    }

    if dry_run.unwrap_or(true) {
        plan.restored = filter_event_secrets(plan.restored);
        return Ok(RestoreEventResponse {
            plan,
            event: None,
            setup_status: None,
        });
    }

    if plan
        .issues
        .iter()
        .any(|issue| issue.severity == RestoreIssueSeverity::Blocking)
    {
        let issues = serde_json::to_string(&plan.issues).unwrap_or_else(|_| "[]".to_string());
        return Err(TauriFunctionError::new(&format!(
            "restore blocked by plan issues: {issues}"
        )));
    }

    let event = app
        .upsert_event(
            plan.restored.clone(),
            Some(version_type.unwrap_or(VersionType::Patch)),
            Some(true),
        )
        .await?;

    // Mirror of the upsert command's sink sync, with pat/oauth None so the
    // stored PAT and tokens survive the restore. Forward-only: a failed
    // sink update never rolls the restore back. No archive prune on
    // desktop — history here is the user's disk.
    let mode = register_sink.unwrap_or(SinkRegistration::Keep);
    let setup_status =
        match sync_local_sink(&handler, &app_id, &event, offline, None, None, mode).await {
            Ok(()) => None,
            Err(e) => {
                tracing::warn!(
                    "Failed to update sink registration of event {} after restore: {}",
                    event.id,
                    e
                );
                Some(format!("error: {e}"))
            }
        };

    plan.restored = filter_event_secrets(plan.restored);
    Ok(RestoreEventResponse {
        plan,
        event: Some(filter_event_secrets(event)),
        setup_status,
    })
}

#[tauri::command(async)]
pub async fn delete_event(
    handler: AppHandle,
    app_id: String,
    event_id: String,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Ok(mut app) = App::load(app_id.clone(), flow_like_state).await {
        if app
            .get_event(&event_id, None)
            .await
            .is_ok_and(|event| event.event_type == "ontology_action")
        {
            return Err(TauriFunctionError::new(
                "Ontology action events are removed through Data Studio actions",
            ));
        }
        // Unregister from sink manager first if registered
        if let Ok(event_sink_manager) =
            crate::state::TauriEventSinkManagerState::construct(&handler).await
        {
            let manager = event_sink_manager.lock().await;
            if let Err(e) = manager.unregister_event(&handler, &event_id).await {
                tracing::warn!(
                    "Failed to unregister event {} from sink manager: {}",
                    event_id,
                    e
                );
                // Continue with deletion even if unregistration fails
            }
        }

        app.delete_event(&event_id).await?;
        return Ok(());
    }

    Err(TauriFunctionError::new("Failed to delete event"))
}

#[tauri::command(async)]
pub async fn validate_event(
    handler: AppHandle,
    app_id: String,
    event_id: String,
    version: Option<(u32, u32, u32)>,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Ok(app) = App::load(app_id.clone(), flow_like_state).await {
        app.validate_event(&event_id, version).await?;
        return Ok(());
    }

    Err(TauriFunctionError::new("Failed to validate event"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `LogMeta.event_version` is dotted `major.minor.patch`, while the board
    /// `version` column is `v{major}-{minor}-{patch}`. `version_key` and every
    /// event-version filter must use the dotted form — this fails if the two
    /// formats are ever swapped.
    #[test]
    fn version_key_uses_the_dotted_event_format_not_the_board_format() {
        assert_eq!(dotted_version_key((1, 2, 3)), "1.2.3");
        assert_ne!(dotted_version_key((1, 2, 3)), "v1-2-3");
    }

    #[test]
    fn lance_ids_are_restricted_to_the_allowlist() {
        assert!(is_safe_id("abc-DEF_123"));
        assert!(!is_safe_id(""));
        assert!(!is_safe_id("abc' OR '1'='1"));
        assert!(!is_safe_id("\"event_id\""));
        assert!(!is_safe_id("runs/../other"));
    }
}
