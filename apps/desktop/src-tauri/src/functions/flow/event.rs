use flow_like::{
    app::{App, AppVisibility},
    flow::{
        board::{Board, VersionType},
        event::{
            Event, EventInput, ReleaseNotes, RestoreIssueSeverity, RestoreOptions, RestorePlan,
            filter_event_secrets,
        },
        execution::{LogMeta, run_index::RunQuery},
        oauth::OAuthToken,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;

use crate::{
    event_sink::{
        EventSinkManager, has_token,
        manager::{
            LocalTriggerPlan, LocalTriggerState, MissingTriggerToken, TriggerSignIn, TriggerUpdate,
            app_is_offline,
        },
        trigger_token::{TriggerAccount, verify_token_owner},
    },
    functions::TauriFunctionError,
    state::{TauriEventSinkManagerState, TauriFlowLikeState},
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

/// Inputs of stored events built by an older `EventInput::from_pin`, rebuilt once per process
/// by (app, event, version) and never written back, so a read creates no version and nothing
/// to sync. `None`: the event's board or node does not resolve here, the stored inputs are
/// served.
type RebuiltInputs = HashMap<(String, String, (u32, u32, u32)), Option<Vec<EventInput>>>;
static REBUILT_INPUTS: LazyLock<Mutex<RebuiltInputs>> = LazyLock::new(Mutex::default);

/// Boards opened by one read, `None` when one does not load.
type ReadBoards = HashMap<(String, Option<(u32, u32, u32)>), Option<Arc<Board>>>;

async fn rebuild_inputs(
    app: &App,
    boards: &mut ReadBoards,
    event: &Event,
) -> Option<Vec<EventInput>> {
    if event.board_id.is_empty() {
        return None;
    }
    let key = (event.board_id.clone(), event.board_version);
    if !boards.contains_key(&key) {
        // Unregistered: a background read must not pin the board open for the session.
        let board = match app
            .open_board(event.board_id.clone(), Some(false), event.board_version)
            .await
        {
            Ok(board) => Some(board.snapshot()),
            Err(error) => {
                tracing::warn!(
                    event_id = %event.id,
                    board_id = %event.board_id,
                    board_version = ?event.board_version,
                    %error,
                    "Outdated event inputs cannot be rebuilt; serving the stored ones"
                );
                None
            }
        };
        boards.insert(key.clone(), board);
    }
    let inputs = event.inputs_from_board(boards.get(&key)?.as_ref()?);
    if inputs.is_none() {
        tracing::debug!(
            event_id = %event.id,
            node_id = %event.node_id,
            "Event node is not on its board; serving the stored inputs"
        );
    }
    inputs
}

/// `event` with its inputs rebuilt in memory when they were stored by an older rule: resolved
/// help and schema, pin options, sensitive defaults withheld.
async fn with_current_inputs(app: &App, boards: &mut ReadBoards, mut event: Event) -> Event {
    if !event.inputs_outdated() {
        return event;
    }
    let key = (app.id.clone(), event.id.clone(), event.event_version);
    let cached = REBUILT_INPUTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
        .cloned();
    let inputs = match cached {
        Some(inputs) => inputs,
        None => {
            let inputs = rebuild_inputs(app, boards, &event).await;
            REBUILT_INPUTS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key, inputs.clone());
            inputs
        }
    };
    if let Some(inputs) = inputs {
        event.inputs = inputs;
    }
    event
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
        return Ok(with_current_inputs(&app, &mut ReadBoards::new(), event).await);
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
        let mut boards = ReadBoards::new();

        for event in events {
            if let Ok(loaded_event) = Event::load(event, &app, None).await {
                if loaded_event.event_type != "ontology_action" {
                    loaded_events.push(with_current_inputs(&app, &mut boards, loaded_event).await);
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

/// A device-only event never keeps a trigger here, whatever the caller asked.
pub(crate) fn effective_sink_mode(event: &Event, requested: SinkRegistration) -> SinkRegistration {
    if event.is_device_source() {
        SinkRegistration::Skip
    } else {
        requested
    }
}

/// Applies `mode` for the saved `event`. `pat` and `oauth_tokens` merge into
/// the registration's stored credentials.
async fn sync_local_sink(
    handler: &AppHandle,
    app_id: &str,
    event: &Event,
    pat: Option<String>,
    oauth_tokens: Option<HashMap<String, OAuthToken>>,
    mode: SinkRegistration,
) -> anyhow::Result<()> {
    let mode = effective_sink_mode(event, mode);
    let manager_state = match TauriEventSinkManagerState::construct(handler).await {
        Ok(manager_state) => manager_state,
        Err(err) if event.is_device_source() => {
            tracing::warn!(
                event_id = %event.id,
                error = %err,
                "Sink manager unavailable, nothing to remove for a device-only event"
            );
            return Ok(());
        }
        Err(err) => return Err(err),
    };
    let update = match mode {
        SinkRegistration::Register => TriggerUpdate::Register,
        SinkRegistration::Keep => TriggerUpdate::Keep,
        SinkRegistration::Skip => {
            return manager_state
                .lock()
                .await
                .decline_local_registration(handler, app_id, &event.id)
                .await;
        }
    };
    update_local_trigger(
        handler,
        &manager_state,
        app_id,
        event,
        pat,
        oauth_tokens,
        update,
    )
    .await
}

/// Resolves who the trigger signs in as before locking the sink manager, so
/// checking a new PAT or clearing a location trigger with the hub never
/// blocks the triggers that fire meanwhile, then updates the trigger under
/// the lock.
async fn update_local_trigger(
    handler: &AppHandle,
    manager_state: &flow_like_types::sync::Mutex<EventSinkManager>,
    app_id: &str,
    event: &Event,
    pat: Option<String>,
    oauth_tokens: Option<HashMap<String, OAuthToken>>,
    update: TriggerUpdate,
) -> anyhow::Result<()> {
    let sign_in = TriggerSignIn::resolve(
        manager_state,
        handler,
        app_id,
        event,
        pat,
        oauth_tokens.as_ref(),
        update,
    )
    .await?;
    manager_state
        .lock()
        .await
        .register_from_flow_event(handler, app_id, event, sign_in, oauth_tokens, update)
        .await
}

/// A saved event with what the save did to its trigger on this device. The
/// event's fields stay at the top level, so callers that read the event
/// keep working. A trigger that failed never fails the committed save; its
/// error is reported here instead.
#[derive(Serialize, Debug, Clone)]
pub struct EventSaveOutcome {
    #[serde(flatten)]
    pub event: Event,
    pub trigger_state: LocalTriggerState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_error: Option<String>,
}

/// The sentence a save reports when its trigger update failed. A refresh
/// that only lacks a token reports nothing: the event's status says so.
fn trigger_error(event: &Event, mode: SinkRegistration, error: &anyhow::Error) -> Option<String> {
    if mode == SinkRegistration::Keep && error.is::<MissingTriggerToken>() {
        tracing::debug!(event_id = %event.id, error = %error, "Trigger refresh skipped");
        return None;
    }
    tracing::warn!(
        event_id = %event.id,
        error = %error,
        "Failed to update the sink registration of a saved event"
    );
    Some(match mode {
        SinkRegistration::Skip => {
            format!(
                "Event was saved, but its trigger on this computer could not be removed: {error}"
            )
        }
        _ if event.event_type == "geolocation" => {
            format!("Location Event was saved, but device monitoring could not be updated: {error}")
        }
        SinkRegistration::Register => {
            format!("Event was saved, but its trigger on this computer was not registered: {error}")
        }
        SinkRegistration::Keep => {
            format!(
                "Event was saved, but its trigger on this computer could not be updated: {error}"
            )
        }
    })
}

/// Syncs the trigger of the saved `event` and reports what that did. Never
/// fails: the save is already committed.
async fn sync_saved_event(
    handler: &AppHandle,
    app_id: &str,
    event: Event,
    offline: bool,
    pat: Option<String>,
    oauth_tokens: Option<HashMap<String, OAuthToken>>,
    mode: SinkRegistration,
) -> EventSaveOutcome {
    let mode = effective_sink_mode(&event, mode);
    let trigger_error = sync_local_sink(handler, app_id, &event, pat, oauth_tokens, mode)
        .await
        .err()
        .and_then(|error| trigger_error(&event, mode, &error));
    let trigger_state = trigger_state(handler, app_id, &event, offline)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(
                event_id = %event.id,
                error = %error,
                "Could not read the trigger state of a saved event"
            );
            LocalTriggerState::None
        });
    EventSaveOutcome {
        event,
        trigger_state,
        trigger_error,
    }
}

async fn trigger_state(
    handler: &AppHandle,
    app_id: &str,
    event: &Event,
    offline: bool,
) -> anyhow::Result<LocalTriggerState> {
    if EventSinkManager::local_registration_blocker(event).is_some() {
        return Ok(LocalTriggerState::None);
    }
    let account = TriggerAccount::current(handler).await;
    let manager_state = TauriEventSinkManagerState::construct(handler).await?;
    let manager = manager_state.lock().await;
    manager.trigger_state(event, app_id, offline, account.as_ref())
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub async fn upsert_event(
    handler: AppHandle,
    app_id: String,
    event: Event,
    version_type: Option<VersionType>,
    enforce_id: Option<bool>,
    pat: Option<String>,
    oauth_tokens: Option<HashMap<String, OAuthToken>>,
    register_sink: Option<SinkRegistration>,
) -> Result<EventSaveOutcome, TauriFunctionError> {
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
        let offline = matches!(app.visibility, AppVisibility::Offline);
        let mode = register_sink.unwrap_or(SinkRegistration::Keep);
        return Ok(
            sync_saved_event(&handler, &app_id, event, offline, pat, oauth_tokens, mode).await,
        );
    }

    Err(TauriFunctionError::new("Failed to upsert event"))
}

/// What saving `event` would do to this device's trigger. Saving reconciles
/// the execution mode with the board, so a stale client copy must not decide
/// whether the user is asked.
async fn trigger_plan(
    handler: &AppHandle,
    app_id: &str,
    mut event: Event,
) -> anyhow::Result<LocalTriggerPlan> {
    let mut offline = false;
    if let Ok(state) = TauriFlowLikeState::construct(handler).await
        && let Ok(app) = App::load(app_id.to_string(), state).await
    {
        let _ = event.reconcile_execution_mode_with_board(&app).await;
        offline = matches!(app.visibility, AppVisibility::Offline);
    }
    if EventSinkManager::local_registration_blocker(&event).is_some() {
        return Ok(LocalTriggerPlan::NONE);
    }
    let account = TriggerAccount::current(handler).await;
    let manager_state = TauriEventSinkManagerState::construct(handler).await?;
    let manager = manager_state.lock().await;
    manager.registration_plan(&event, app_id, offline, account.as_ref())
}

/// Whether saving `event` registers a trigger on this device and whether the
/// user has to choose a token for it first.
#[tauri::command(async)]
pub async fn local_trigger_plan(
    handler: AppHandle,
    app_id: String,
    event: Event,
) -> Result<LocalTriggerPlan, TauriFunctionError> {
    Ok(trigger_plan(&handler, &app_id, event).await?)
}

/// This device's trigger for `event` of `app_id`.
#[tauri::command(async)]
pub async fn local_trigger_status(
    handler: AppHandle,
    app_id: String,
    event: Event,
) -> Result<LocalTriggerState, TauriFunctionError> {
    let offline = app_is_offline(&handler, &app_id).await;
    Ok(trigger_state(&handler, &app_id, &event, offline).await?)
}

/// Repairs this device's trigger of a stored event without saving the event:
/// `register` signs the existing trigger in with `pat`, `skip` removes it.
#[tauri::command(async)]
pub async fn sync_local_trigger(
    handler: AppHandle,
    app_id: String,
    event_id: String,
    register_sink: SinkRegistration,
    pat: Option<String>,
) -> Result<LocalTriggerState, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id.clone(), flow_like_state)
        .await
        .map_err(|_| TauriFunctionError::new("App not found"))?;
    let event = app.get_event(&event_id, None).await?;
    let update = repair_update(effective_sink_mode(&event, register_sink), pat.as_deref())?;
    repair_local_trigger(&handler, &app_id, &event, update, pat).await?;
    let offline = matches!(app.visibility, AppVisibility::Offline);
    Ok(trigger_state(&handler, &app_id, &event, offline).await?)
}

/// Signs the trigger of `event` in again with `pat`, or removes it when
/// `update` is `None`.
async fn repair_local_trigger(
    handler: &AppHandle,
    app_id: &str,
    event: &Event,
    update: Option<TriggerUpdate>,
    pat: Option<String>,
) -> anyhow::Result<()> {
    let Some(update) = update else {
        return sync_local_sink(handler, app_id, event, None, None, SinkRegistration::Skip).await;
    };
    let manager_state = TauriEventSinkManagerState::construct(handler).await?;
    update_local_trigger(handler, &manager_state, app_id, event, pat, None, update).await
}

/// A repair only signs this app's existing trigger of the same type and
/// destination in with a new token, or removes it (`None`); creating or
/// converting a trigger takes a save.
fn repair_update(
    mode: SinkRegistration,
    pat: Option<&str>,
) -> Result<Option<TriggerUpdate>, TauriFunctionError> {
    match mode {
        SinkRegistration::Skip => Ok(None),
        SinkRegistration::Keep => Err(TauriFunctionError::new(
            "A trigger repair either registers with a token or skips the trigger",
        )),
        SinkRegistration::Register if !has_token(pat) => Err(TauriFunctionError::new(
            "Choose a personal access token to repair this trigger",
        )),
        SinkRegistration::Register => Ok(Some(TriggerUpdate::Repair)),
    }
}

/// Checks that `pat` belongs to the signed-in account on the current hub and
/// may run events in `app_id`, before anything is saved with it.
#[tauri::command(async)]
pub async fn verify_trigger_token(
    handler: AppHandle,
    app_id: String,
    pat: String,
) -> Result<(), TauriFunctionError> {
    verify_token_owner(&handler, &app_id, &pat).await?;
    Ok(())
}

/// Wire twin of the API's `POST /apps/{app_id}/events/{event_id}/restore`
/// response. `plan.restored` (and `event`) are serialized with secret variable
/// values blanked — the plan never carries a secret.
#[derive(Serialize, Debug, Clone)]
pub struct RestoreEventResponse {
    pub plan: RestorePlan,
    /// The event as persisted, with what restoring did to its trigger on this
    /// device — present only after a non-dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<EventSaveOutcome>,
    /// Outcome of the non-fatal sink re-registration after a non-dry run. A
    /// failure here does not roll the restore back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_status: Option<String>,
}

impl RestoreEventResponse {
    /// The response for `plan` and, after a non-dry run, the `outcome` of
    /// saving the restored event, with secret variable values blanked.
    fn redacted(mut plan: RestorePlan, outcome: Option<EventSaveOutcome>) -> Self {
        plan.restored = filter_event_secrets(plan.restored);
        let setup_status = outcome
            .as_ref()
            .and_then(|outcome| outcome.trigger_error.as_ref())
            .map(|error| format!("error: {error}"));
        Self {
            plan,
            event: outcome.map(|outcome| EventSaveOutcome {
                event: filter_event_secrets(outcome.event),
                ..outcome
            }),
            setup_status,
        }
    }
}

fn refuse_ontology_action(event: &Event) -> Result<(), TauriFunctionError> {
    if event.event_type == "ontology_action" {
        return Err(TauriFunctionError::new(
            "Ontology action events are managed through Data Studio actions",
        ));
    }
    Ok(())
}

fn refuse_blocking_issues(plan: &RestorePlan) -> Result<(), TauriFunctionError> {
    if !plan
        .issues
        .iter()
        .any(|issue| issue.severity == RestoreIssueSeverity::Blocking)
    {
        return Ok(());
    }
    let issues = serde_json::to_string(&plan.issues).unwrap_or_else(|_| "[]".to_string());
    Err(TauriFunctionError::new(&format!(
        "restore blocked by plan issues: {issues}"
    )))
}

/// The plan for restoring `version` over the live event `event_id`.
async fn restore_plan(
    app: &App,
    event_id: &str,
    version: (u32, u32, u32),
    options: &RestoreOptions,
) -> Result<RestorePlan, TauriFunctionError> {
    let live = app.get_event(event_id, None).await?;
    refuse_ontology_action(&live)?;
    let plan = Event::plan_restore(app, event_id, version, &live, options).await?;
    refuse_ontology_action(&plan.restored)?;
    Ok(plan)
}

/// Saves the event `plan` restores and syncs its trigger like
/// [`upsert_event`]. Without a new `pat` and with no OAuth tokens, the stored
/// credentials survive the restore. Forward-only: a failed trigger update
/// never rolls the restore back. No archive prune on desktop — history here
/// is the user's disk.
async fn apply_restore(
    handler: &AppHandle,
    app: &mut App,
    app_id: &str,
    plan: &RestorePlan,
    version_type: VersionType,
    pat: Option<String>,
    mode: SinkRegistration,
) -> Result<EventSaveOutcome, TauriFunctionError> {
    refuse_blocking_issues(plan)?;
    let event = app
        .upsert_event(plan.restored.clone(), Some(version_type), Some(true))
        .await?;
    let offline = matches!(app.visibility, AppVisibility::Offline);
    Ok(sync_saved_event(handler, app_id, event, offline, pat, None, mode).await)
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
    register_sink: Option<SinkRegistration>,
    pat: Option<String>,
) -> Result<RestoreEventResponse, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let mut app = App::load(app_id.clone(), flow_like_state)
        .await
        .map_err(|_| TauriFunctionError::new("App not found"))?;
    let options = RestoreOptions {
        restore_route: restore_route.unwrap_or(false),
        drop_canary: drop_canary.unwrap_or(false),
        accept_blank_secrets: accept_blank_secrets.unwrap_or(false),
    };
    let plan = restore_plan(&app, &event_id, version, &options).await?;
    if dry_run.unwrap_or(true) {
        return Ok(RestoreEventResponse::redacted(plan, None));
    }
    let outcome = apply_restore(
        &handler,
        &mut app,
        &app_id,
        &plan,
        version_type.unwrap_or(VersionType::Patch),
        pat,
        register_sink.unwrap_or(SinkRegistration::Keep),
    )
    .await?;
    Ok(RestoreEventResponse::redacted(plan, Some(outcome)))
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
        if let Ok(event_sink_manager) = TauriEventSinkManagerState::construct(&handler).await {
            let manager = event_sink_manager.lock().await;
            if let Err(e) = manager.remove_trigger(&handler, &app_id, &event_id).await {
                tracing::warn!(
                    "Failed to remove the trigger of event {} from this device: {}",
                    event_id,
                    e
                );
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

    fn saved_event(event_type: &str) -> Event {
        let now = SystemTime::now();
        Event {
            id: "event".to_string(),
            name: "Event".to_string(),
            description: String::new(),
            board_id: "board".to_string(),
            board_version: None,
            node_id: "node".to_string(),
            variables: HashMap::new(),
            config: Vec::new(),
            active: true,
            canary: None,
            variants: Vec::new(),
            priority: 0,
            event_type: event_type.to_string(),
            notes: None,
            event_version: (1, 0, 0),
            created_at: now,
            updated_at: now,
            default_page_id: None,
            inputs: Vec::new(),
            route: None,
            is_default: false,
            execution_mode: Default::default(),
            exposure: Default::default(),
            correlation_mappings: None,
        }
    }

    #[test]
    fn upsert_outcome_flattens_the_event() {
        let ready = serde_json::to_value(EventSaveOutcome {
            event: saved_event("cron"),
            trigger_state: LocalTriggerState::Ready,
            trigger_error: None,
        })
        .unwrap();
        assert_eq!(ready["id"], "event");
        assert_eq!(ready["event_type"], "cron");
        assert_eq!(ready["trigger_state"], "ready");
        assert!(ready.get("trigger_error").is_none());
        assert!(ready.get("event").is_none());

        let failed = serde_json::to_value(EventSaveOutcome {
            event: saved_event("cron"),
            trigger_state: LocalTriggerState::Failed {
                reason: "sink refused".to_string(),
            },
            trigger_error: Some("Event was saved, but …".to_string()),
        })
        .unwrap();
        assert_eq!(
            failed["trigger_state"],
            serde_json::json!({ "failed": { "reason": "sink refused" } })
        );
        assert_eq!(failed["trigger_error"], "Event was saved, but …");

        let event: Event = serde_json::from_value(failed).unwrap();
        assert_eq!(event.id, "event");
    }

    #[test]
    fn trigger_errors_name_what_the_save_could_not_do() {
        let cron = saved_event("cron");
        let missing: anyhow::Error = MissingTriggerToken("event".to_string()).into();
        assert_eq!(trigger_error(&cron, SinkRegistration::Keep, &missing), None);
        assert!(
            trigger_error(&cron, SinkRegistration::Register, &missing)
                .unwrap()
                .starts_with(
                    "Event was saved, but its trigger on this computer was not registered: "
                )
        );

        let failure = anyhow::anyhow!("sink refused");
        assert_eq!(
            trigger_error(&cron, SinkRegistration::Skip, &failure).as_deref(),
            Some(
                "Event was saved, but its trigger on this computer could not be removed: sink refused"
            )
        );
        assert_eq!(
            trigger_error(&cron, SinkRegistration::Keep, &failure).as_deref(),
            Some(
                "Event was saved, but its trigger on this computer could not be updated: sink refused"
            )
        );
        assert_eq!(
            trigger_error(
                &saved_event("geolocation"),
                SinkRegistration::Register,
                &failure
            )
            .as_deref(),
            Some(
                "Location Event was saved, but device monitoring could not be updated: sink refused"
            )
        );
    }

    #[test]
    fn a_repair_signs_the_trigger_in_with_a_token_or_removes_it() {
        assert!(matches!(
            repair_update(SinkRegistration::Skip, None),
            Ok(None)
        ));
        assert!(matches!(
            repair_update(SinkRegistration::Register, Some("pat_1")),
            Ok(Some(TriggerUpdate::Repair))
        ));
        assert!(repair_update(SinkRegistration::Register, Some("  ")).is_err());
        assert!(repair_update(SinkRegistration::Register, None).is_err());
        assert!(repair_update(SinkRegistration::Keep, Some("pat_1")).is_err());
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
