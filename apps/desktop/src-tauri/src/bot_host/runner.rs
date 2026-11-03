//! Runs a turn through the same native path as background actions: no window, bot tools of this
//! desktop only, and every question or approval answered "unavailable".

use super::host::{RunError, TurnJob, TurnRunner};
use crate::{
    functions::ai::{
        copilot::{cancel_copilot_chat, global_chat_native},
        native_tool_bridge::{
            NativeInteractionMode, NativeRunIdentity, NativeRunOptions, with_execution_profile,
        },
    },
    state::{TauriFlowLikeState, TauriSettingsState},
};
use async_trait::async_trait;
use flow_like::copilot::CopilotScope;
use std::time::Duration;
use tauri::{AppHandle, Manager, ipc::Channel};

const SESSION_POLL: Duration = Duration::from_millis(500);
const CODE_RUN_FAILED: &str = "BOT_TURN_FAILED";
const CODE_SETUP: &str = "BOT_TURN_SETUP";

pub struct NativeRunner(pub AppHandle);

fn setup(message: impl Into<String>) -> RunError {
    RunError::Failed {
        code: CODE_SETUP.into(),
        message: message.into(),
    }
}

/// The bot's profile when it still exists, else the active one; the session is the one signed in
/// to that profile's hub right now (none when signed out).
pub(super) async fn identity(
    app: &AppHandle,
    profile_id: Option<&str>,
) -> Result<NativeRunIdentity, String> {
    let settings = TauriSettingsState::construct(app)
        .await
        .map_err(|error| error.to_string())?;
    let settings = settings.lock().await;
    let profile = match profile_id.and_then(|id| settings.profile_by_id(id).ok()) {
        Some(profile) => profile,
        None => settings
            .get_current_profile()
            .map_err(|error| error.to_string())?,
    };
    let hub = if profile.hub_profile.hub.trim().is_empty() {
        String::new()
    } else {
        flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure)
            .ok_or("The bot's profile has an invalid hub origin.")?
    };
    let auth = crate::native_auth::SessionBinding::capture(&profile.hub_profile.id, &hub).await?;
    Ok(NativeRunIdentity {
        profile_id: profile.hub_profile.id,
        hub,
        subject: auth.as_ref().map(|binding| binding.subject.clone()),
        auth,
    })
}

#[async_trait]
impl TurnRunner for NativeRunner {
    async fn run(&self, job: TurnJob) -> Result<String, RunError> {
        let app = &self.0;
        let identity = identity(app, job.profile_id.as_deref())
            .await
            .map_err(setup)?;
        let state = app
            .try_state::<TauriFlowLikeState>()
            .ok_or_else(|| setup("Flow Like is still starting."))?;
        let profile_id = identity.profile_id.clone();
        let options = NativeRunOptions {
            profile_id: Some(profile_id.clone()),
            expected_identity: Some(identity.clone()),
            conversation_id: Some(job.conversation_id.clone()),
            bot_memory: job.memory.map(|memory| memory.0),
            approval_gate: Some(job.gate.clone()),
            requester: job.requester.clone(),
            interaction_mode: NativeInteractionMode::Unavailable,
            ..Default::default()
        };
        let run = with_execution_profile(
            profile_id,
            global_chat_native(
                app.clone(),
                None,
                state,
                CopilotScope::Both,
                job.prompt,
                None,
                Some(job.history),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(job.run_id.clone()),
                Channel::<String>::new(|_| Ok(())),
                options,
                Some(job.bot_id),
            ),
        );
        tokio::pin!(run);
        let mut poll = tokio::time::interval(SESSION_POLL);
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                result = &mut run => {
                    return result.map(|response| response.message).map_err(|message| {
                        RunError::Failed { code: CODE_RUN_FAILED.into(), message }
                    });
                }
                _ = poll.tick() => {
                    let current = identity.auth.as_ref().is_none_or(|binding| binding.is_current());
                    if !current {
                        let _ = cancel_copilot_chat(job.run_id.clone());
                        return Err(RunError::SessionChanged);
                    }
                }
            }
        }
    }

    fn cancel(&self, run_id: &str) {
        let _ = cancel_copilot_chat(run_id.to_owned());
    }
}
