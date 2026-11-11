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
use flow_like::flow::copilot::decision;
use flow_like::flow_like_model_provider::systemone::{SystemOneRequest, SystemOneResponse};
use flow_like::models::llm::ModelUsageContext;
use serde_json::json;
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
    async fn classify(
        &self,
        job: &TurnJob,
        request: &SystemOneRequest,
    ) -> Result<Option<SystemOneResponse>, RunError> {
        use super::hub::TurnHub;
        use crate::functions::local_bots::{LocalBots, on_store};

        let app = &self.0;
        let identity = identity(app, job.profile_id.as_deref())
            .await
            .map_err(setup)?;
        let local = match app.try_state::<LocalBots>() {
            Some(store) => {
                let id = job.bot_id.clone();
                on_store(&store, move |store| store.get_bot(&id))
                    .await
                    .map_err(|e| setup(e.to_string()))?
            }
            None => None,
        };
        let record = match local {
            Some(bot) => json!({
                "model": bot.model, "name": bot.name, "instructions": bot.instructions,
                "location": "desktop",
            }),
            None => super::hub::ProfileHub::new(app.clone())
                .read(&format!("bots/{}", job.bot_id), Vec::new())
                .await
                .map_err(|e| setup(e.to_string()))?,
        };
        let Some(model) = record["model"]["decision"]
            .as_str()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            return Ok(None);
        };
        let settings = TauriSettingsState::construct(app)
            .await
            .map_err(|e| setup(e.to_string()))?;
        let profile = {
            let settings = settings.lock().await;
            let mut profile = settings
                .profile_by_id(&identity.profile_id)
                .map_err(|e| setup(e.to_string()))?
                .hub_profile;
            profile.custom_bits = settings
                .custom_bits
                .iter()
                .cloned()
                .map(flow_like::profile::ProfileCustomBit)
                .collect();
            profile
        };
        let state = app
            .try_state::<TauriFlowLikeState>()
            .ok_or_else(|| setup("Flow Like is still starting."))?
            .0
            .clone();
        let bit = profile
            .find_bit(model, state.http_client.clone())
            .await
            .map_err(|e| setup(e.to_string()))?;
        let parameters = bit
            .try_to_systemone()
            .ok_or_else(|| setup("The bot's decision model is not a SystemOne Bit"))?;
        if matches!(record["location"].as_str(), Some("server" | "device"))
            && !flow_like::flow_like_model_provider::provider::is_hosted_provider_name(
                &parameters.provider.provider_name,
            )
        {
            return Err(setup(
                "Server and managed-device bots require a hosted decision model",
            ));
        }
        let token = match (&job.hub_token, identity.auth.as_ref()) {
            (Some(token), _) => Some(token.0.clone()),
            (None, Some(auth)) => Some(auth.cached_access_token().await.map_err(setup)?),
            (None, None) => None,
        };
        let client = state
            .model_factory
            .build_systemone(
                &bit,
                state.clone(),
                token,
                Some(ModelUsageContext {
                    run_id: Some(job.run_id.clone()),
                    api_base_url: (!identity.hub.is_empty()).then_some(identity.hub.clone()),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| setup(e.to_string()))?;
        let mut request = request.clone();
        request.state["context"]["bot"] = json!({
            "name": record["name"].as_str().map(|name| decision::excerpt(name, 120)),
            "instructions": record["instructions"].as_str().map(|text| decision::excerpt(text, 2000)),
        });
        if identity
            .auth
            .as_ref()
            .is_some_and(|auth| !auth.is_current())
        {
            return Err(RunError::SessionChanged);
        }
        let response = client
            .invoke(&request)
            .await
            .map_err(|e| setup(e.to_string()))?;
        if identity
            .auth
            .as_ref()
            .is_some_and(|auth| !auth.is_current())
        {
            return Err(RunError::SessionChanged);
        }
        response
            .validate_for(&request)
            .map_err(|e| setup(e.to_string()))?;
        Ok(Some(response))
    }

    async fn run(&self, job: TurnJob) -> Result<String, RunError> {
        let app = &self.0;
        let identity = identity(app, job.profile_id.as_deref())
            .await
            .map_err(setup)?;
        if identity
            .auth
            .as_ref()
            .is_some_and(|auth| !auth.is_current())
        {
            return Err(RunError::SessionChanged);
        }
        if let Some(recovery) = &job.recovery
            && let Some(checkpoint) = recovery
                .load()
                .await
                .map_err(|error| setup(error.to_string()))?
            && let Some(answer) =
                flow_like::flow::copilot::run_recovery::finished_reply(&checkpoint)
                    .map_err(|error| setup(error.to_string()))?
        {
            if identity
                .auth
                .as_ref()
                .is_some_and(|auth| !auth.is_current())
            {
                return Err(RunError::SessionChanged);
            }
            return Ok(answer);
        }
        let state = app
            .try_state::<TauriFlowLikeState>()
            .ok_or_else(|| setup("Flow Like is still starting."))?;
        let profile_id = identity.profile_id.clone();
        let options = NativeRunOptions {
            recovery: job.recovery.clone(),
            profile_id: Some(profile_id.clone()),
            expected_identity: Some(identity.clone()),
            conversation_id: Some(job.conversation_id.clone()),
            inbox_conversation_id: Some(job.conversation_id.clone()),
            bot_memory: job.memory.map(|memory| memory.0),
            notes: job.notes.clone().filter(|_| !job.acknowledgement),
            trail: job.trail.clone().filter(|_| !job.acknowledgement),
            reply_files: job.reply_files.clone().filter(|_| !job.acknowledgement),
            room_updates: job.room_updates.clone(),
            approval_gate: Some(job.gate.clone()),
            requester: job.requester.clone(),
            acknowledgement: job.acknowledgement,
            live_call: job.live_call,
            tool_gate: job.tool_gate.clone(),
            ephemeral_user_context: job.ephemeral_user_context,
            shared_context: job.shared_context.clone(),
            suppress_owner_notifications: job.suppress_owner_notifications,
            answered_round: job.answered_round.clone(),
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
