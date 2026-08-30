use flow_like::{
    app::App,
    flow::{board::VersionType, event::Event, oauth::OAuthToken},
};
use std::collections::HashMap;
use tauri::AppHandle;

use crate::{functions::TauriFunctionError, state::TauriFlowLikeState};

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

        // Automatically register/update the event with the sink manager if applicable
        match crate::state::TauriEventSinkManagerState::construct(&handler).await {
            Ok(event_sink_manager) => {
                let manager = event_sink_manager.lock().await;
                if let Err(e) = manager
                    .register_from_flow_event(&handler, &app_id, &event, offline, pat, oauth_tokens)
                    .await
                {
                    println!(
                        "Failed to auto-register event {} with sink manager: {}",
                        event.id, e
                    );
                    tracing::warn!(
                        "Failed to auto-register event {} with sink manager: {}",
                        event.id,
                        e
                    );
                    // Don't fail the entire upsert if sink registration fails
                }
            }
            Err(e) => {
                println!(
                    "EventSinkManager not available (may still be initializing): {}",
                    e
                );
                tracing::warn!(
                    "EventSinkManager not available (may still be initializing): {}",
                    e
                );
                tracing::warn!(
                    "Event {} will need to be registered with sink manager later",
                    event.id
                );
            }
        }

        return Ok(event);
    }

    Err(TauriFunctionError::new("Failed to upsert event"))
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
