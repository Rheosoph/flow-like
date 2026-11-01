//! Runs result-bearing Apple actions without a webview or a frontend queue consumer.

use crate::{
    functions::ai::{
        copilot::{cancel_copilot_chat, global_chat_native},
        native_tool_bridge::{NativeRunIdentity, NativeRunOptions, with_execution_profile},
    },
    state::{TauriFlowLikeState, TauriSettingsState},
};
use flow_like::copilot::CopilotScope;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    future::Future,
    sync::{Arc, LazyLock, Mutex as IdentityMutex},
    time::Duration,
};
use tauri::{AppHandle, Manager, ipc::Channel};
use tokio::sync::{Mutex, Semaphore};

const MAX_DURATION_MS: i64 = 25_000;
const CANCELLED: &str = "The background FlowPilot request was cancelled or its workspace changed.";
const TIMED_OUT: &str =
    "FlowPilot reached the background action time limit. Try a shorter request.";
static DRAIN: Mutex<()> = Mutex::const_new(());
static RUN_SLOTS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(4));

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActionRequest {
    id: String,
    scope: String,
    action: Action,
    created_at: String,
    response_mode: String,
    response_deadline: String,
    execution_mode: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Action {
    kind: String,
    prompt: Option<String>,
    #[serde(default)]
    voice: bool,
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Debug)]
struct Scope {
    origin: String,
    profile_id: String,
    subject: String,
}

impl Scope {
    fn parse(raw: &str) -> Result<Self, String> {
        if raw.len() > 8192 {
            return Err("The action workspace is invalid.".into());
        }
        let [origin, profile_id, subject]: [String; 3] =
            serde_json::from_str(raw).map_err(|_| "The action workspace is invalid.")?;
        let url = reqwest::Url::parse(&origin).map_err(|_| "The action hub is invalid.")?;
        if profile_id.is_empty()
            || subject.is_empty()
            || !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("The action workspace is invalid.".into());
        }
        Ok(Self {
            origin: origin.trim_end_matches('/').into(),
            profile_id,
            subject,
        })
    }

    fn matches(&self, identity: &NativeRunIdentity) -> bool {
        self.profile_id == identity.profile_id
            && self.origin == identity.hub
            && self.subject == identity.subject.as_deref().unwrap_or("local")
    }
}

impl ActionRequest {
    fn validate(&self, now: i64) -> Result<(Scope, Duration), String> {
        if uuid::Uuid::parse_str(&self.id).is_err()
            || self.execution_mode != "background"
            || self.action.kind != "flowpilot"
            || self.response_mode != "text"
        {
            return Err("This action does not support background execution.".into());
        }
        if self.action.voice {
            return Err("Start voice in Flow Like. Background actions return text.".into());
        }
        if !self.action.files.is_empty() {
            return Err("This background action does not accept shared files.".into());
        }
        if !self
            .action
            .prompt
            .as_ref()
            .is_some_and(|prompt| !prompt.trim().is_empty() && prompt.len() <= 65_536)
        {
            return Err("Provide a question no larger than 64 KiB.".into());
        }
        let created = timestamp(&self.created_at)?;
        let deadline = timestamp(&self.response_deadline)?;
        if created > now.saturating_add(60_000)
            || created <= now.saturating_sub(300_000)
            || deadline <= now
            || deadline <= created
            || deadline > created.saturating_add(MAX_DURATION_MS)
        {
            return Err("The background action expired or has an invalid deadline.".into());
        }
        let remaining = deadline.saturating_sub(now).min(MAX_DURATION_MS) as u64;
        Ok((Scope::parse(&self.scope)?, Duration::from_millis(remaining)))
    }
}

fn timestamp(value: &str) -> Result<i64, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|time| time.timestamp_millis())
        .map_err(|_| "The background action timestamp is invalid.".into())
}

/// The Swift store atomically removes only native-owned requests before returning them.
/// A process restart must not replay a request that may already have performed a mutation.
pub(super) fn notify(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _drain = DRAIN.lock().await;
        let actions = match super::apple::claim_background_actions() {
            Ok(Value::Array(actions)) => actions,
            Ok(_) => {
                tracing::warn!("Native background action queue returned an invalid response");
                return;
            }
            Err(_) => {
                tracing::warn!("Native background action queue is unavailable");
                return;
            }
        };
        for value in actions {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { process(app, value).await });
        }
    });
}

async fn resolve_identity(app: &AppHandle, scope: &Scope) -> Result<NativeRunIdentity, String> {
    let settings = TauriSettingsState::construct(app)
        .await
        .map_err(|_| "The action workspace is unavailable.")?;
    let settings = settings.lock().await;
    let profile = settings
        .profile_by_id(&scope.profile_id)
        .map_err(|_| "The action profile no longer exists.")?;
    let hub = flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure)
        .ok_or("The action profile has no hub.")?;
    if hub != scope.origin {
        return Err("The action profile's hub changed. Set up the action again.".into());
    }
    let auth = crate::native_auth::SessionBinding::capture(&scope.profile_id, &hub).await?;
    let identity = NativeRunIdentity {
        profile_id: scope.profile_id.clone(),
        hub,
        subject: auth.as_ref().map(|binding| binding.subject.clone()),
        auth,
    };
    if !scope.matches(&identity) {
        return Err(
            "The action account changed. Open Flow Like and set up the action again.".into(),
        );
    }
    Ok(identity)
}

async fn process(app: AppHandle, value: Value) {
    let id = value.get("id").and_then(Value::as_str).unwrap_or("");
    let scope = value.get("scope").and_then(Value::as_str).unwrap_or("");
    let request: ActionRequest = match serde_json::from_value(value.clone()) {
        Ok(request) => request,
        Err(_) => {
            let _ = super::apple::complete_action(action_result(
                id,
                scope,
                Err("The background action request is invalid.".into()),
            ));
            return;
        }
    };
    let result = match request.validate(chrono::Utc::now().timestamp_millis()) {
        Err(error) => Err(error),
        Ok((scope, remaining)) => {
            let deadline = tokio::time::Instant::now() + remaining;
            let run_id = format!("native-action-{}", uuid::Uuid::new_v4());
            let task_run_id = run_id.clone();
            let captured = Arc::new(IdentityMutex::new(None::<NativeRunIdentity>));
            let task_identity = captured.clone();
            let work = async {
                // Waiting for a slot consumes the caller's existing deadline.
                let _slot = RUN_SLOTS
                    .acquire()
                    .await
                    .map_err(|_| "Background execution is unavailable.")?;
                let identity = resolve_identity(&app, &scope).await?;
                *task_identity
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(identity.clone());
                let profile_id = identity.profile_id.clone();
                let options = NativeRunOptions {
                    profile_id: Some(profile_id.clone()),
                    expected_identity: Some(identity.clone()),
                    conversation_id: Some(task_run_id.clone()),
                    ..Default::default()
                };
                let state = app
                    .try_state::<TauriFlowLikeState>()
                    .ok_or("FlowPilot is still starting. Try again shortly.")?;
                let response = with_execution_profile(
                    profile_id,
                    global_chat_native(
                        app.clone(),
                        None,
                        state,
                        CopilotScope::Both,
                        request
                            .action
                            .prompt
                            .as_deref()
                            .unwrap_or_default()
                            .trim()
                            .to_owned(),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some(task_run_id),
                        Channel::<String>::new(|_| Ok(())),
                        options,
                    ),
                )
                .await?;
                if !identity.matches(&resolve_identity(&app, &scope).await?) {
                    return Err(CANCELLED.into());
                }
                Ok(response.message)
            };
            supervise(
                work,
                deadline,
                || {
                    let current = captured
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .as_ref()
                        .and_then(|identity| identity.auth.as_ref())
                        .is_none_or(|binding| binding.is_current());
                    Ok(current
                        && super::apple::background_action_active(&request.id, &request.scope)?)
                },
                || {
                    let _ = cancel_copilot_chat(run_id.clone());
                },
            )
            .await
        }
    };
    // The store rechecks scope, deadline and registration before publishing the result.
    // Cancelled intents remove their registration, so late output cannot be delivered.
    let _ = super::apple::complete_action(action_result(&request.id, &request.scope, result));
}

async fn supervise<T>(
    work: impl Future<Output = Result<T, String>>,
    deadline: tokio::time::Instant,
    mut active: impl FnMut() -> Result<bool, String>,
    mut cancel: impl FnMut(),
) -> Result<T, String> {
    if !active()? {
        return Err(CANCELLED.into());
    }
    tokio::pin!(work);
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => {
                cancel();
                return Err(TIMED_OUT.into());
            }
            _ = poll.tick() => {
                match active() {
                    Ok(true) => {},
                    _ => {
                        cancel();
                        return Err(CANCELLED.into());
                    }
                }
            }
            result = &mut work => {
                if !active().unwrap_or(false) {
                    cancel();
                    return Err(CANCELLED.into());
                }
                return result;
            }
        }
    }
}

fn action_result(id: &str, scope: &str, result: Result<String, String>) -> Value {
    let (status, text, error) = match result {
        Ok(text) if text.trim().is_empty() => (
            "interaction_required",
            None,
            Some("FlowPilot did not return a text answer. Open Flow Like to continue.".into()),
        ),
        Ok(text) if text.len() <= 196_608 => ("success", Some(text), None),
        Ok(_) => (
            "error",
            None,
            Some("The FlowPilot reply exceeds the action result limit.".into()),
        ),
        Err(error) => {
            let mut end = error.len().min(4096);
            while !error.is_char_boundary(end) {
                end -= 1;
            }
            ("error", None, Some(error[..end].to_owned()))
        }
    };
    let result = json!({"id":id,"scope":scope,"status":status,"text":text,"error":error});
    if result.to_string().len() > 262_144 {
        return json!({"id":id,"scope":scope,"status":"error","error":"The FlowPilot reply exceeds the action result limit."});
    }
    result
}

#[cfg(test)]
#[path = "native_actions_tests.rs"]
mod tests;
