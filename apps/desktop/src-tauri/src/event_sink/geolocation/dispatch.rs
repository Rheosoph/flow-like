use super::{
    model::{GeoLocationSink, Scope, Transition},
    native,
};
use crate::{
    event_bus::EventBusEvent,
    event_sink::{
        EventRegistration, has_token,
        manager::Refusal,
        trigger_token::{TokenRefused, authorization, hub_client, read_body},
    },
    state::{TauriFlowLikeState, TauriSettingsState},
};
use anyhow::{Result, anyhow, ensure};
use flow_like::{
    app::{App, AppStatus, AppVisibility},
    flow::{
        event::{Event, EventExecutionMode},
        execution::UserExecutionContext,
        oauth::OAuthToken,
    },
    state::FlowLikeState,
};
use flow_like_types::{OAuthTokenInput, intercom::BufferedInterComHandler};
use serde::de::DeserializeOwned;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::AppHandle;

/// Longest reason kept from a hub refusal.
const REASON_LIMIT: usize = 300;
const UNVERIFIED_TOKEN: &str = "Hosted geofences require a personal access token verified with the hub; add one to the trigger in the event's settings";

#[derive(Debug)]
pub struct Rejected(pub String);
impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Rejected {}

/// The refusal behind a failed dispatch, when retrying cannot help.
pub fn refusal(error: &anyhow::Error) -> Option<Refusal> {
    if let Some(Rejected(reason)) = error.downcast_ref::<Rejected>() {
        return Some(Refusal {
            reason: reason.clone(),
            token_refused: false,
        });
    }
    error
        .downcast_ref::<TokenRefused>()
        .map(|TokenRefused(reason)| Refusal {
            reason: reason.clone(),
            token_refused: true,
        })
}

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
    let settings = TauriSettingsState::construct(app).await?;
    let profile = settings.lock().await.profile_by_id(&scope.profile_id)?;
    require!(
        flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure).as_deref()
            == Some(scope.origin.as_str()),
        "The geofence profile's hub changed"
    );
    let binding = crate::native_auth::SessionBinding::capture(&scope.profile_id, &scope.origin)
        .await
        .map_err(anyhow::Error::msg)?;
    require!(
        binding
            .as_ref()
            .map(|binding| binding.subject.as_str())
            .unwrap_or("local")
            == scope.subject,
        "The geofence profile's signed-in account changed"
    );
    Ok(scope)
}

pub(super) fn registration_matches_scope(registration: &EventRegistration, scope: &Scope) -> bool {
    registration
        .profile_binding
        .as_ref()
        .is_some_and(|binding| {
            binding.profile_id == scope.profile_id
                && binding.hub_origin == scope.origin
                && binding.subject.as_deref().unwrap_or("local") == scope.subject
        })
}

async fn json_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    // Authentication failures never fall back to locally cached Event authority.
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(TokenRefused(format!(
            "The hub no longer accepts this trigger's personal access token ({status})"
        ))
        .into());
    }
    if status.is_client_error() && status.as_u16() != 408 && status.as_u16() != 429 {
        return Err(Rejected(format!("Geofence API request was rejected ({status})")).into());
    }
    ensure!(
        status.is_success(),
        "Geofence API request failed ({status})"
    );
    Ok(serde_json::from_slice(&read_body(response).await?)?)
}

/// How a crossing forwarded to the hub's geofence door ended.
#[derive(Debug, PartialEq, Eq)]
enum DoorAnswer {
    Accepted,
    /// The hub will not run this event from this device; it is declined.
    Refused,
    /// The hub refused the PAT itself.
    TokenRefused,
    /// Worth delivering again with backoff.
    Retry,
}

/// Of the 409s only EVENT_PAUSED declines the region. Any other, such as
/// PROJECT_INACTIVE or IDEMPOTENCY_KEY_IN_USE, has the crossing sent again
/// and keeps the region, so the trigger resumes once its project is active.
fn door_answer(status: u16, code: Option<&str>) -> DoorAnswer {
    match status {
        200..=299 => DoorAnswer::Accepted,
        401 => DoorAnswer::TokenRefused,
        409 if code == Some("EVENT_PAUSED") => DoorAnswer::Refused,
        402 | 408 | 409 | 425 | 429 => DoorAnswer::Retry,
        400..=499 => DoorAnswer::Refused,
        _ => DoorAnswer::Retry,
    }
}

/// The `{"error":{"code","message"}}` envelope of a hub error, if the body is one.
fn hub_error(body: &[u8]) -> (Option<String>, Option<String>) {
    let value: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    let field = |name: &str| {
        value["error"][name]
            .as_str()
            .map(|text| text.chars().take(REASON_LIMIT).collect())
    };
    (field("code"), field("message"))
}

/// Hands a crossing of a Remote location region to the hub, which runs the
/// event as the account the region is registered for, with the OAuth sign-ins
/// this trigger holds for that account and, for any other provider, the
/// event's saved sign-ins. The transition id makes redelivery idempotent.
async fn forward(
    scope: &Scope,
    registration: &EventRegistration,
    transition: &Transition,
) -> Result<()> {
    let token = registration
        .personal_access_token
        .as_deref()
        .filter(|token| has_token(Some(*token)))
        .ok_or_else(|| anyhow!("Missing Event authentication"))?;
    let response = hub_client()?
        .post(format!(
            "{}/api/v1/sink/trigger/geofence/{}",
            scope.origin,
            urlencoding::encode(&registration.event_id)
        ))
        .header("Authorization", authorization(token))
        .header("Idempotency-Key", &transition.id)
        .json(&crossing(transition, &registration.oauth_tokens, scope))
        .send()
        .await?;
    let status = response.status().as_u16();
    let body = read_body(response).await?;
    let (code, message) = hub_error(&body);
    match door_answer(status, code.as_deref()) {
        DoorAnswer::Accepted => {
            let accepted: serde_json::Value = serde_json::from_slice(&body)?;
            ensure!(
                accepted["run_id"].as_str().is_some_and(|id| !id.is_empty()),
                "The hub did not start a run for the geofence crossing"
            );
            Ok(())
        }
        DoorAnswer::TokenRefused => Err(TokenRefused(format!(
            "The hub no longer accepts this trigger's personal access token ({status})"
        ))
        .into()),
        DoorAnswer::Refused => Err(Rejected(format!(
            "The hub refused the location trigger ({status} {}): {}",
            code.as_deref().unwrap_or("ERROR"),
            message.as_deref().unwrap_or("no reason given")
        ))
        .into()),
        DoorAnswer::Retry => Err(anyhow!(
            "The hub could not take the geofence crossing yet ({status} {}): {}",
            code.as_deref().unwrap_or("ERROR"),
            message.as_deref().unwrap_or("no reason given")
        )),
    }
}

/// What the hub's geofence door takes for a crossing: the transition, and the
/// trigger's sign-ins `tokens` when it holds any it may send there.
fn crossing(
    transition: &Transition,
    tokens: &HashMap<String, OAuthToken>,
    scope: &Scope,
) -> serde_json::Value {
    let mut body = serde_json::json!({ "transition": transition });
    let sign_ins = hub_sign_ins(scope, tokens);
    if !sign_ins.is_empty() {
        body["oauth_tokens"] = serde_json::json!(sign_ins);
    }
    body
}

/// The trigger's OAuth sign-ins in the form an event invocation takes them,
/// keyed by provider: only those bound to `scope`'s hub and account, which the
/// hub resolves as that account's. An expired one that cannot be renewed stays
/// behind, so the event's saved sign-in serves its provider instead.
fn hub_sign_ins(
    scope: &Scope,
    tokens: &HashMap<String, OAuthToken>,
) -> HashMap<String, OAuthTokenInput> {
    tokens
        .iter()
        .filter(|(_, token)| token.ensure_hub(&scope.origin, true).is_ok())
        .filter(|(_, token)| {
            token
                .hub_subject
                .as_deref()
                .is_none_or(|subject| subject == scope.subject)
        })
        .filter(|(_, token)| token.refresh_token.is_some() || !token.is_expired())
        .map(|(provider, token)| {
            let sign_in = OAuthTokenInput {
                access_token: token.access_token.clone(),
                refresh_token: token.refresh_token.clone(),
                token_type: token.token_type.clone(),
                expires_at: token.expires_at.and_then(|at| i64::try_from(at).ok()),
            };
            (provider.clone(), sign_in)
        })
        .collect()
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

/// What the registered account may do with the event behind `registration`:
/// an offline app answers on this device, any other app on the hub. Asking
/// the hub is a round trip, so registering a trigger resolves this before
/// the sink manager is locked (see [`super::GeofenceClearance`]).
pub async fn authorize(
    app: &AppHandle,
    registration: &EventRegistration,
    scope: &Scope,
) -> Result<Authority> {
    require_scope(app, scope).await?;
    require!(
        registration_matches_scope(registration, scope),
        "This geofence has no matching profile binding; turn its trigger on again"
    );
    registration
        .profile_binding
        .as_ref()
        .expect("checked profile binding")
        .resolve(app)
        .await?;
    let local_app = active_local_app(app, &registration.app_id).await?;
    if let Some(local) = local_app
        .as_ref()
        .filter(|local| matches!(local.visibility, AppVisibility::Offline))
    {
        return offline_authority(local, registration, scope).await;
    }
    let authority = hub_authority(registration, scope).await?;
    if authority.event.execution_mode == EventExecutionMode::Local {
        require_local_copy(app, local_app, &authority, scope).await?;
    }
    require_scope(app, scope).await?;
    Ok(authority)
}

async fn require_scope(app: &AppHandle, scope: &Scope) -> Result<()> {
    require!(
        current_scope(app).await?.raw == scope.raw,
        "Geofence account changed"
    );
    Ok(())
}

/// The app of a geofence as downloaded to this device, if it is; it must be
/// active.
async fn active_local_app(app: &AppHandle, app_id: &str) -> Result<Option<App>> {
    let state = TauriFlowLikeState::construct(app).await?;
    let local_app = App::load(app_id.to_string(), state).await.ok();
    if let Some(local) = local_app.as_ref() {
        require_active(app_id, &local.status)?;
    }
    Ok(local_app)
}

/// An app that is not active holds its crossings back, which is no refusal:
/// its region stays watched and the trigger resumes once the app is active.
fn require_active(app_id: &str, status: &AppStatus) -> Result<()> {
    let state = match status {
        AppStatus::Active => return Ok(()),
        AppStatus::Inactive => "inactive",
        AppStatus::Archived => "archived",
    };
    Err(anyhow!("Geofence app {app_id} is {state}"))
}

/// An offline app runs its Local geofence Event as the device's account.
async fn offline_authority(
    local: &App,
    registration: &EventRegistration,
    scope: &Scope,
) -> Result<Authority> {
    let event = local.get_event(&registration.event_id, None).await?;
    event_matches(&event, registration, scope)?;
    require!(
        event.execution_mode == EventExecutionMode::Local,
        "Offline geofences require a Local Event"
    );
    Ok(Authority {
        event,
        identity: UserExecutionContext::local(&scope.subject),
        offline: true,
    })
}

/// Who the trigger's verified PAT is on the hub, and the event as the hub
/// has it; the account must be the one the region is registered for.
async fn hub_authority(registration: &EventRegistration, scope: &Scope) -> Result<Authority> {
    require!(
        scope.subject != "local",
        "Hosted geofences require the registered account to be signed in"
    );
    let token = verified_token(registration)?;
    let http = hub_client()?;
    let root = format!(
        "{}/api/v1/apps/{}",
        scope.origin,
        urlencoding::encode(&registration.app_id)
    );
    let identity: UserExecutionContext =
        hub_get(&http, &format!("{root}/invoke/context"), token).await?;
    require!(
        runs_events_as_registered(&identity, scope),
        "The current account cannot execute this geofence Event"
    );
    let event_url = format!(
        "{root}/events/{}",
        urlencoding::encode(&registration.event_id)
    );
    let event: Event = hub_get(&http, &event_url, token).await?;
    event_matches(&event, registration, scope)?;
    require!(
        (event.execution_mode == EventExecutionMode::Remote) == registration.remote,
        "Geofence Event moved between this device and the hub; turn its trigger on again"
    );
    Ok(Authority {
        event,
        identity,
        offline: false,
    })
}

fn verified_token(registration: &EventRegistration) -> Result<&str> {
    registration
        .personal_access_token
        .as_deref()
        .filter(|_| registration.has_verified_token())
        .ok_or_else(|| anyhow!(UNVERIFIED_TOKEN))
}

/// Whether the hub's `identity` is the person the region is registered for,
/// and may run events.
fn runs_events_as_registered(identity: &UserExecutionContext, scope: &Scope) -> bool {
    !identity.is_technical_user && identity.sub == scope.subject && identity.has_permission(0x2000)
}

async fn hub_get<T: DeserializeOwned>(http: &reqwest::Client, url: &str, token: &str) -> Result<T> {
    json_response(
        http.get(url)
            .header("Authorization", authorization(token))
            .send()
            .await?,
    )
    .await
}

/// A hosted Local event runs on this device: the account needs the board
/// rights, this device the current download of the event, and the profile
/// the account's hub.
async fn require_local_copy(
    app: &AppHandle,
    local_app: Option<App>,
    authority: &Authority,
    scope: &Scope,
) -> Result<()> {
    let (event, identity) = (&authority.event, &authority.identity);
    require!(
        identity.has_permission(0x100) && identity.has_permission(0x200),
        "Hosted Local geofences require ReadBoards and ExecuteBoards"
    );
    let local = local_app
        .ok_or_else(|| anyhow!("Download this app before enabling a Local geofence Event"))?;
    let cached = local.get_event(&event.id, None).await?;
    require!(
        runs_the_same_board(&cached, event),
        "Download the current Event before running its geofence locally"
    );
    let settings = TauriSettingsState::construct(app).await?;
    let profile = settings.lock().await.profile_by_id(&scope.profile_id)?;
    require!(
        flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure).as_deref()
            == Some(scope.origin.as_str()),
        "The Local geofence profile must use the current account's hub origin"
    );
    Ok(())
}

/// Whether the downloaded `cached` event runs the board node and version the
/// hub's `event` runs.
fn runs_the_same_board(cached: &Event, event: &Event) -> bool {
    cached.board_id == event.board_id
        && cached.board_version == event.board_version
        && cached.node_id == event.node_id
        && cached.event_version == event.event_version
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
    require!(
        current_scope(app).await?.raw == scope.raw,
        "Geofence account changed"
    );
    if authority.event.execution_mode == EventExecutionMode::Remote {
        return forward(scope, registration, transition).await;
    }
    let payload = serde_json::to_value(transition)?;
    let state = TauriFlowLikeState::construct(app).await?;
    let run_id = Arc::new(Mutex::new(None));
    let _guard = LocalRunGuard {
        state: state.clone(),
        id: run_id.clone(),
    };
    EventBusEvent {
        payload: Some(payload),
        app_id: registration.app_id.clone(),
        event_id: registration.event_id.clone(),
        offline: authority.offline,
        token: registration.personal_access_token.clone(),
        token_owner: registration.token_owner(),
        profile_binding: registration.profile_binding.clone(),
        callback: Some(run_callback(app, run_id)),
        request_control: None,
        oauth_tokens: registration.oauth_tokens.clone(),
    }
    .execute_authorized(app, state, authority.event, authority.identity)
    .await?;
    Ok(())
}

/// Shows the events of a local geofence run in the UI and records its run
/// id, so the run is cancelled when the dispatch is interrupted.
fn run_callback(
    app: &AppHandle,
    run_id: Arc<Mutex<Option<String>>>,
) -> Arc<BufferedInterComHandler> {
    let handle = app.clone();
    BufferedInterComHandler::new(
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
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn door_answers_split_into_runs_refusals_and_retries() {
        let cases = [
            (200, None, DoorAnswer::Accepted),
            (202, None, DoorAnswer::Accepted),
            (400, Some("BAD_REQUEST"), DoorAnswer::Refused),
            (401, Some("UNAUTHORIZED"), DoorAnswer::TokenRefused),
            (402, Some("QUOTA_EXCEEDED"), DoorAnswer::Retry),
            (403, Some("SINK_RUN_AS_REFUSED"), DoorAnswer::Refused),
            (403, Some("FORBIDDEN"), DoorAnswer::Refused),
            (404, Some("NOT_FOUND"), DoorAnswer::Refused),
            (408, None, DoorAnswer::Retry),
            (409, Some("EVENT_PAUSED"), DoorAnswer::Refused),
            (409, Some("PROJECT_INACTIVE"), DoorAnswer::Retry),
            (409, Some("IDEMPOTENCY_KEY_IN_USE"), DoorAnswer::Retry),
            (409, Some("CONFLICT"), DoorAnswer::Retry),
            (409, None, DoorAnswer::Retry),
            (429, None, DoorAnswer::Retry),
            (500, None, DoorAnswer::Retry),
            (502, None, DoorAnswer::Retry),
            (503, None, DoorAnswer::Retry),
        ];
        for (status, code, answer) in cases {
            assert_eq!(door_answer(status, code), answer, "{status} {code:?}");
        }
    }

    #[test]
    fn only_refusals_decline_and_only_a_refused_token_is_dropped() {
        let rejected: anyhow::Error = Rejected("paused".into()).into();
        assert_eq!(
            refusal(&rejected),
            Some(Refusal {
                reason: "paused".into(),
                token_refused: false,
            })
        );
        let revoked: anyhow::Error = TokenRefused("revoked".into()).into();
        assert_eq!(
            refusal(&revoked),
            Some(Refusal {
                reason: "revoked".into(),
                token_refused: true,
            })
        );
        assert_eq!(refusal(&anyhow!("connection timed out")), None);
    }

    #[test]
    fn an_app_that_is_not_active_holds_crossings_back_without_a_refusal() {
        assert!(require_active("app-1", &AppStatus::Active).is_ok());
        for (status, state) in [
            (AppStatus::Inactive, "inactive"),
            (AppStatus::Archived, "archived"),
        ] {
            let held = require_active("app-1", &status).unwrap_err();
            assert_eq!(held.to_string(), format!("Geofence app app-1 is {state}"));
            assert_eq!(refusal(&held), None, "{state}");
        }
    }

    #[test]
    fn hub_error_envelopes_give_their_code_and_message() {
        let (code, message) =
            hub_error(br#"{"error":{"code":"EVENT_PAUSED","message":"The event is paused"}}"#);
        assert_eq!(code.as_deref(), Some("EVENT_PAUSED"));
        assert_eq!(message.as_deref(), Some("The event is paused"));
        assert_eq!(hub_error(b"<html>bad gateway</html>"), (None, None));
    }

    fn sign_in(
        refresh_token: Option<&str>,
        hub_origin: Option<&str>,
        hub_subject: Option<&str>,
        expires_at: Option<u64>,
    ) -> OAuthToken {
        let mut token = OAuthToken::new("access".into());
        token.refresh_token = refresh_token.map(str::to_owned);
        token.hub_origin = hub_origin.map(str::to_owned);
        token.hub_subject = hub_subject.map(str::to_owned);
        token.expires_at = expires_at;
        token
    }

    fn crossing_of(id: &str) -> Transition {
        Transition {
            id: id.into(),
            registration_id: "registration-1".into(),
            scope: "scope".into(),
            app_id: "app-1".into(),
            event_id: "event-1".into(),
            transition: "enter".into(),
            occurred_at: 1,
            geometry: serde_json::json!({"type": "Point", "coordinates": [0.0, 0.0]}),
            radius_meters: 50.0,
        }
    }

    /// A crossing carries the trigger's sign-ins bound to the hub and account its region is
    /// registered for, in the form an event invocation takes them, so the hub resolves them
    /// as that account's. Sign-ins of another hub or account stay on the device, and so does
    /// an expired one that cannot be renewed; the event's saved sign-ins serve those providers.
    #[test]
    fn a_crossing_sends_only_the_sign_ins_of_its_hub_and_account() {
        let scope = Scope::parse(r#"["https://hub.example","profile-1","ada"]"#.into()).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let hub = Some("https://hub.example");
        let tokens = HashMap::from([
            (
                "google".to_owned(),
                sign_in(Some("fl_oauth_google"), hub, Some("ada"), Some(now + 3600)),
            ),
            (
                "slack".to_owned(),
                sign_in(None, hub, None, Some(now + 3600)),
            ),
            (
                "expired".to_owned(),
                sign_in(None, hub, Some("ada"), Some(now)),
            ),
            (
                "renewable".to_owned(),
                sign_in(Some("fl_oauth_renewable"), hub, Some("ada"), Some(now)),
            ),
            (
                "other-hub".to_owned(),
                sign_in(
                    Some("fl_oauth_other"),
                    Some("https://other.example"),
                    Some("ada"),
                    None,
                ),
            ),
            (
                "other-account".to_owned(),
                sign_in(Some("fl_oauth_bob"), hub, Some("bob"), None),
            ),
            (
                "unbound".to_owned(),
                sign_in(Some("fl_oauth_unbound"), None, None, None),
            ),
        ]);
        let sent = hub_sign_ins(&scope, &tokens);
        let mut providers: Vec<&str> = sent.keys().map(String::as_str).collect();
        providers.sort_unstable();
        assert_eq!(providers, ["google", "renewable", "slack"]);

        let body = crossing(&crossing_of("crossing-1"), &tokens, &scope);
        let mut sent_providers: Vec<&str> = body["oauth_tokens"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        sent_providers.sort_unstable();
        assert_eq!(sent_providers, providers);
        assert_eq!(body["transition"]["id"], "crossing-1");
        assert_eq!(
            body["oauth_tokens"]["google"],
            serde_json::json!({
                "access_token": "access",
                "refresh_token": "fl_oauth_google",
                "token_type": "Bearer",
                "expires_at": now + 3600,
            })
        );
        assert_eq!(
            body["oauth_tokens"]["slack"],
            serde_json::json!({
                "access_token": "access",
                "token_type": "Bearer",
                "expires_at": now + 3600,
            })
        );

        let elsewhere = HashMap::from([("other-hub".to_owned(), tokens["other-hub"].clone())]);
        for none_to_send in [HashMap::new(), elsewhere] {
            let bare = crossing(&crossing_of("crossing-2"), &none_to_send, &scope);
            assert_eq!(bare["transition"]["id"], "crossing-2");
            assert!(bare.get("oauth_tokens").is_none());
        }
    }
}
