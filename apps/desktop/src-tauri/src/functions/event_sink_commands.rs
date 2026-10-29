use crate::event_sink::{
    EventConfig, EventRegistration, EventSinkManager, has_token, manager::LocalTriggerState,
    trigger_token::TriggerAccount,
};
use crate::state::TauriEventSinkManagerState;
use std::time::SystemTime;
use tauri::AppHandle;
use tracing::instrument;

#[derive(Debug, serde::Serialize)]
pub struct TauriFunctionError {
    pub message: String,
}

impl From<String> for TauriFunctionError {
    fn from(message: String) -> Self {
        TauriFunctionError { message }
    }
}

impl From<anyhow::Error> for TauriFunctionError {
    fn from(error: anyhow::Error) -> Self {
        TauriFunctionError {
            message: error.to_string(),
        }
    }
}

/// A registration as the webview sees it: whether it holds credentials, never
/// their values.
#[derive(Debug, serde::Serialize)]
pub struct EventRegistrationView {
    pub event_id: String,
    pub name: String,
    pub r#type: String,
    pub updated_at: SystemTime,
    pub created_at: SystemTime,
    pub config: EventConfig,
    pub offline: bool,
    pub remote: bool,
    pub app_id: String,
    pub default_payload: Option<flow_like_types::Value>,
    pub has_token: bool,
    pub pat_owner_sub: Option<String>,
    pub oauth_providers: Vec<String>,
    pub state: LocalTriggerState,
}

fn view(
    manager: &EventSinkManager,
    registration: EventRegistration,
    account: Option<&TriggerAccount>,
) -> anyhow::Result<EventRegistrationView> {
    let state = manager.registration_state(&registration, account)?;
    let mut oauth_providers: Vec<String> = registration.oauth_tokens.into_keys().collect();
    oauth_providers.sort();
    Ok(EventRegistrationView {
        has_token: has_token(registration.personal_access_token.as_deref()),
        event_id: registration.event_id,
        name: registration.name,
        r#type: registration.r#type,
        updated_at: registration.updated_at,
        created_at: registration.created_at,
        config: registration.config,
        offline: registration.offline,
        remote: registration.remote,
        app_id: registration.app_id,
        default_payload: registration.default_payload,
        pat_owner_sub: registration.pat_owner_sub,
        oauth_providers,
        state,
    })
}

/// Remove an event sink and forget its credentials. With `app_id`, only a
/// trigger of that app is removed.
#[instrument(skip(app_handle))]
#[tauri::command(async)]
pub async fn remove_event_sink(
    app_handle: AppHandle,
    event_id: String,
    app_id: Option<String>,
) -> Result<(), TauriFunctionError> {
    let manager_arc = TauriEventSinkManagerState::construct(&app_handle)
        .await
        .map_err(|e| format!("Failed to get EventSinkManager: {}", e))?;

    let manager = manager_arc.lock().await;
    manager
        .remove_event_sink(&app_handle, app_id.as_deref(), &event_id)
        .await
        .map_err(|e| e.into())
}

/// Get a specific event sink registration
#[instrument(skip(app_handle))]
#[tauri::command(async)]
pub async fn get_event_sink(
    app_handle: AppHandle,
    event_id: String,
) -> Result<Option<EventRegistrationView>, TauriFunctionError> {
    let account = TriggerAccount::current(&app_handle).await;
    let manager_arc = TauriEventSinkManagerState::construct(&app_handle)
        .await
        .map_err(|e| format!("Failed to get EventSinkManager: {}", e))?;

    let manager = manager_arc.lock().await;
    let registration = manager.get_registration(&event_id)?;
    Ok(registration
        .map(|registration| view(&manager, registration, account.as_ref()))
        .transpose()?)
}

/// List all event sink registrations
#[instrument(skip(app_handle))]
#[tauri::command(async)]
pub async fn list_event_sinks(
    app_handle: AppHandle,
) -> Result<Vec<EventRegistrationView>, TauriFunctionError> {
    let account = TriggerAccount::current(&app_handle).await;
    let manager_arc = TauriEventSinkManagerState::construct(&app_handle)
        .await
        .map_err(|e| format!("Failed to get EventSinkManager: {}", e))?;

    let manager = manager_arc.lock().await;
    let registrations = manager.list_registrations()?;
    Ok(registrations
        .into_iter()
        .map(|registration| view(&manager, registration, account.as_ref()))
        .collect::<anyhow::Result<Vec<_>>>()?)
}

/// Check if a sink is active for an event
/// Returns true if the event is registered and its trigger can fire as the
/// signed-in account
#[instrument(skip(app_handle))]
#[tauri::command(async)]
pub async fn is_event_sink_active(
    app_handle: AppHandle,
    event_id: String,
) -> Result<bool, TauriFunctionError> {
    let account = TriggerAccount::current(&app_handle).await;
    let manager_arc = TauriEventSinkManagerState::construct(&app_handle)
        .await
        .map_err(|e| format!("Failed to get EventSinkManager: {}", e))?;

    let manager = manager_arc.lock().await;
    Ok(manager.is_event_active(&event_id, account.as_ref()))
}
