use super::{
    model::{GeoLocationSink, Scope, Transition},
    native,
};
use crate::{
    event_bus::EventBusEvent,
    event_sink::EventRegistration,
    state::{TauriFlowLikeState, TauriSettingsState},
};
use anyhow::{Result, anyhow, ensure};
use flow_like::{
    app::{App, AppStatus, AppVisibility},
    flow::{
        event::{Event, EventExecutionMode},
        execution::UserExecutionContext,
    },
    state::FlowLikeState,
};
use flow_like_types::intercom::BufferedInterComHandler;
use serde::de::DeserializeOwned;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::AppHandle;

#[derive(Debug)]
pub struct Rejected(pub String);
impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Rejected {}
macro_rules! require {
    ($condition:expr, $message:expr) => {
        if !$condition {
            return Err(Rejected($message.into()).into());
        }
    };
}

pub struct Authority {
    pub event: Event,
    pub identity: UserExecutionContext,
    pub offline: bool,
}

pub async fn current_scope(app: &AppHandle) -> Result<Scope> {
    let scope = Scope::parse(native::scope()?)?;
    let profile = TauriSettingsState::current_profile(app).await?;
    require!(
        profile.hub_profile.id == scope.profile_id,
        "Geofence belongs to another profile"
    );
    Ok(scope)
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(18))
        .build()?)
}

fn authorization(token: &str) -> String {
    if token.starts_with("pat_") || token.starts_with("Bearer ") {
        token.into()
    } else {
        format!("Bearer {token}")
    }
}

async fn json_response<T: DeserializeOwned>(mut response: reqwest::Response) -> Result<T> {
    // Authentication failures never fall back to locally cached Event authority.
    let status = response.status();
    if status.is_client_error() && status.as_u16() != 408 && status.as_u16() != 429 {
        return Err(Rejected(format!("Geofence API request was rejected ({status})")).into());
    }
    ensure!(
        status.is_success(),
        "Geofence API request failed ({status})"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 1024 * 1024,
            "Geofence API response exceeds 1 MiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn event_matches(event: &Event, registration: &EventRegistration, scope: &Scope) -> Result<()> {
    require!(
        event.id == registration.event_id && event.active && event.event_type == "geolocation",
        "Geofence Event is inactive, missing, or has changed type"
    );
    let config: GeoLocationSink = serde_json::from_slice(&event.config)
        .map_err(|_| Rejected("Geofence Event configuration is invalid".into()))?;
    config
        .validate()
        .map_err(|error| Rejected(error.to_string()))?;
    let crate::event_sink::EventConfig::GeoLocation(registered) = &registration.config else {
        return Err(anyhow!("Event registration is not a geofence"));
    };
    require!(
        config
            .registration(&scope.raw, &registration.app_id, &registration.event_id)
            .id
            == registered
                .registration(&scope.raw, &registration.app_id, &registration.event_id)
                .id,
        "Geofence Event configuration changed; update its device registration"
    );
    Ok(())
}

pub async fn authorize(
    app: &AppHandle,
    registration: &EventRegistration,
    scope: &Scope,
) -> Result<Authority> {
    require!(
        current_scope(app).await?.raw == scope.raw,
        "Geofence account changed"
    );
    let state = TauriFlowLikeState::construct(app).await?;
    let local_app = App::load(registration.app_id.clone(), state).await.ok();
    if let Some(local) = local_app.as_ref() {
        require!(
            matches!(local.status, AppStatus::Active),
            "Geofence app is inactive"
        );
        if matches!(local.visibility, AppVisibility::Offline) {
            let event = local.get_event(&registration.event_id, None).await?;
            event_matches(&event, registration, scope)?;
            require!(
                event.execution_mode == EventExecutionMode::Local,
                "Offline geofences require a Local Event"
            );
            return Ok(Authority {
                event,
                identity: UserExecutionContext::local(&scope.subject),
                offline: true,
            });
        }
    }
    require!(
        scope.subject != "local",
        "Hosted geofences require the registered account to be signed in"
    );
    let token = registration
        .personal_access_token
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("Hosted geofences require a personal access token"))?;
    let http = client()?;
    let app_id = urlencoding::encode(&registration.app_id);
    let root = format!("{}/api/v1/apps/{app_id}", scope.origin);
    let identity: UserExecutionContext = json_response(
        http.get(format!("{root}/invoke/context"))
            .header("Authorization", authorization(token))
            .send()
            .await?,
    )
    .await?;
    require!(
        !identity.is_technical_user
            && identity.sub == scope.subject
            && identity.has_permission(0x2000),
        "The current account cannot execute this geofence Event"
    );
    let event: Event = json_response(
        http.get(format!(
            "{root}/events/{}",
            urlencoding::encode(&registration.event_id)
        ))
        .header("Authorization", authorization(token))
        .send()
        .await?,
    )
    .await?;
    event_matches(&event, registration, scope)?;
    if event.execution_mode == EventExecutionMode::Local {
        require!(
            identity.has_permission(0x100) && identity.has_permission(0x200),
            "Hosted Local geofences require ReadBoards and ExecuteBoards"
        );
        let local = local_app
            .ok_or_else(|| anyhow!("Download this app before enabling a Local geofence Event"))?;
        let cached = local.get_event(&event.id, None).await?;
        require!(
            cached.board_id == event.board_id
                && cached.board_version == event.board_version
                && cached.node_id == event.node_id
                && cached.event_version == event.event_version,
            "Download the current Event before running its geofence locally"
        );
        let profile = TauriSettingsState::current_profile(app).await?;
        require!(
            flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure)
                .as_deref()
                == Some(scope.origin.as_str()),
            "The Local geofence profile must use the current account's hub origin"
        );
    }
    require!(
        current_scope(app).await?.raw == scope.raw,
        "Geofence account changed"
    );
    Ok(Authority {
        event,
        identity,
        offline: false,
    })
}

struct LocalRunGuard {
    state: Arc<FlowLikeState>,
    id: Arc<Mutex<Option<String>>>,
}
impl Drop for LocalRunGuard {
    fn drop(&mut self) {
        if let Some(id) = self.id.lock().unwrap().as_deref() {
            let _ = self.state.remove_and_cancel_run(id);
        }
    }
}

pub async fn execute(
    app: &AppHandle,
    registration: &EventRegistration,
    scope: &Scope,
    transition: &Transition,
) -> Result<()> {
    let authority = authorize(app, registration, scope).await?;
    let payload = serde_json::to_value(transition)?;
    require!(
        current_scope(app).await?.raw == scope.raw,
        "Geofence account changed"
    );
    if authority.event.execution_mode == EventExecutionMode::Remote {
        let token = registration
            .personal_access_token
            .as_deref()
            .ok_or_else(|| anyhow!("Missing Event authentication"))?;
        let response: serde_json::Value = json_response(client()?.post(format!("{}/api/v1/apps/{}/events/{}/invoke/async",scope.origin,urlencoding::encode(&registration.app_id),urlencoding::encode(&registration.event_id)))
            .header("Authorization",authorization(token))
            .json(&serde_json::json!({"payload":payload,"token":token,"oauth_tokens":registration.oauth_tokens,"profile_id":scope.profile_id,"correlation":{"geofence_transition_id":transition.id}}))
            .send().await?).await?;
        ensure!(
            response["run_id"].as_str().is_some_and(|id| !id.is_empty()),
            "The server did not accept a geofence run"
        );
        return Ok(());
    }
    let state = TauriFlowLikeState::construct(app).await?;
    let run_id = Arc::new(Mutex::new(None));
    let _guard = LocalRunGuard {
        state: state.clone(),
        id: run_id.clone(),
    };
    let handle = app.clone();
    let callback = BufferedInterComHandler::new(
        Arc::new(move |events| {
            for event in &events {
                if event.event_type == "run_initiated" {
                    *run_id.lock().unwrap() = event.payload["run_id"].as_str().map(str::to_owned);
                }
            }
            if let Some(first) = events.first() {
                crate::utils::emit_event_batch_throttled(
                    &handle,
                    crate::utils::UiEmitTarget::All,
                    &first.event_type,
                    events.clone(),
                    Duration::from_millis(150),
                );
            }
            Box::pin(async { Ok(()) })
        }),
        Some(1),
        Some(1),
        Some(false),
    );
    EventBusEvent {
        payload: Some(payload),
        app_id: registration.app_id.clone(),
        event_id: registration.event_id.clone(),
        offline: authority.offline,
        token: registration.personal_access_token.clone(),
        callback: Some(callback),
        oauth_tokens: registration.oauth_tokens.clone(),
    }
    .execute_authorized(app, state, authority.event, authority.identity)
    .await?;
    Ok(())
}
