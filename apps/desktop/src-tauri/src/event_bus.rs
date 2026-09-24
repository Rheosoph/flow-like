use crate::{
    state::TauriSettingsState,
    utils::{UiEmitTarget, local_execution_environment},
};
use flow_like::app::App;
use flow_like::flow::event::Event;
use flow_like::flow::execution::rejection::{RejectedRun, RejectionStage};
use flow_like::flow::execution::{InternalRun, LogMeta, UserExecutionContext};
use flow_like::flow::oauth::OAuthToken;
use flow_like::state::RunData;
use flow_like::{flow::execution::RunPayload, state::FlowLikeState};
use flow_like_types::intercom::{BufferedInterComHandler, InterComEvent};
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::{Value, sync::mpsc};
use flow_like_types::{json, tokio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager};

// Maximum number of events to queue. 100,000 should be plenty for local handling.
const MAX_QUEUE_SIZE: usize = 100_000;

/// Update the last_node_update timestamp for a run when we see run events
fn touch_run_last_update(app_handle: &AppHandle, events: &[InterComEvent]) {
    for event in events {
        // Run events have type "run:{run_id}"
        if event.event_type.starts_with("run:") {
            let run_id = &event.event_type[4..]; // Skip "run:" prefix
            if let Some(state) = app_handle.try_state::<crate::state::TauriFlowLikeState>()
                && let Some(run_data) = state.0.board_run_registry.get(run_id)
            {
                run_data.touch_last_node_update();
            }
        }
    }
}

pub struct EventBusEvent {
    pub payload: Option<Value>,
    pub app_id: String,
    pub event_id: String,

    pub offline: bool,

    // Either Access Token or PAT
    pub token: Option<String>,

    pub callback: Option<Arc<BufferedInterComHandler>>,

    /// OAuth tokens for third-party services
    pub oauth_tokens: std::collections::HashMap<String, OAuthToken>,
}

impl EventBusEvent {
    pub async fn execute(
        &self,
        app_handle: &AppHandle,
        flow_like_state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Option<LogMeta>> {
        self.execute_with_authority(app_handle, flow_like_state, None)
            .await
    }

    /// Native geofences resolve current Event and caller authority before unattended execution.
    pub(crate) async fn execute_authorized(
        &self,
        app_handle: &AppHandle,
        flow_like_state: Arc<FlowLikeState>,
        event: Event,
        identity: UserExecutionContext,
    ) -> flow_like_types::Result<Option<LogMeta>> {
        self.execute_with_authority(app_handle, flow_like_state, Some((event, identity)))
            .await
    }

    async fn execute_with_authority(
        &self,
        app_handle: &AppHandle,
        flow_like_state: Arc<FlowLikeState>,
        authority: Option<(Event, UserExecutionContext)>,
    ) -> flow_like_types::Result<Option<LogMeta>> {
        let started = Arc::new(AtomicBool::new(false));
        match self
            .execute_inner(
                app_handle,
                flow_like_state.clone(),
                started.clone(),
                authority,
            )
            .await
        {
            Ok(meta) => Ok(meta),
            Err(error) => {
                if !started.load(Ordering::Relaxed) {
                    self.record_rejection(&flow_like_state, &error).await;
                }
                Err(error)
            }
        }
    }

    /// Schedules and sinks fire unattended, so a setup failure here has no
    /// caller to report to. Give it the same run history a real execution
    /// would have produced, with the reason as its only log line.
    async fn record_rejection(
        &self,
        flow_like_state: &Arc<FlowLikeState>,
        error: &flow_like_types::Error,
    ) {
        let execution_state = Arc::new(flow_like_state.for_execution_run());
        let event = match App::load(self.app_id.clone(), execution_state.clone()).await {
            Ok(app) => app.get_event(&self.event_id, None).await.ok(),
            Err(_) => None,
        };

        let Some(event) = event else {
            tracing::warn!(
                app_id = %self.app_id,
                event_id = %self.event_id,
                error = %error,
                "Event trigger failed before its board could be identified; nothing to record"
            );
            return;
        };

        let rejection = RejectedRun::new(
            self.app_id.clone(),
            event.board_id.clone(),
            RejectionStage::Setup,
            error.to_string(),
        )
        .with_event_definition(&event)
        .with_payload(self.payload.as_ref());

        if let Err(error) = flow_like_state.record_rejected_run(&rejection).await {
            tracing::warn!(
                app_id = %self.app_id,
                event_id = %self.event_id,
                error = %error,
                "Failed to record a rejected event trigger"
            );
        }
    }

    async fn execute_inner(
        &self,
        app_handle: &AppHandle,
        flow_like_state: Arc<FlowLikeState>,
        started: Arc<AtomicBool>,
        authority: Option<(Event, UserExecutionContext)>,
    ) -> flow_like_types::Result<Option<LogMeta>> {
        let execution_state = Arc::new(flow_like_state.for_execution_run());

        let Ok(app) = App::load(self.app_id.clone(), execution_state.clone()).await else {
            return Err(flow_like_types::anyhow!("App not found"));
        };

        let (loaded_event, resolved_identity) = match authority {
            Some((event, identity)) => (event, Some(identity)),
            None => (app.get_event(&self.event_id, None).await?, None),
        };
        let payload = RunPayload {
            id: loaded_event.node_id.clone(),
            payload: self.payload.to_owned(),
            runtime_variables: None, // Event bus triggers don't have runtime vars context
            filter_secrets: Some(false), // Desktop execution is trusted
        };

        let board_version = loaded_event.board_version;
        let board_id = loaded_event.board_id.clone();

        let template = crate::functions::flow::run::resolve_run_template(
            &execution_state,
            &self.app_id,
            &board_id,
            board_version,
        )
        .await?;
        let profile = TauriSettingsState::current_profile(app_handle).await?;

        crate::functions::automation_approval::ensure_automation_approved(
            app_handle,
            &self.app_id,
            &template.board,
            Some(&self.event_id),
            // Background triggers require a remembered grant and cannot spend a manual Run Once.
            false,
            &profile.hub_profile,
        )
        .await?;

        let app_handle_clone = app_handle.clone();
        let buffered_sender = if let Some(callback) = &self.callback {
            callback.clone()
        } else {
            BufferedInterComHandler::new(
                Arc::new(move |event| {
                    let app_handle = app_handle_clone.clone();
                    Box::pin({
                        async move {
                            // Update last_node_update for run events
                            touch_run_last_update(&app_handle, &event);

                            let first_event = event.first();

                            if let Some(first_event) = first_event {
                                crate::utils::emit_event_batch_throttled(
                                    &app_handle,
                                    UiEmitTarget::All,
                                    &first_event.event_type,
                                    event.clone(),
                                    std::time::Duration::from_millis(150),
                                );
                            }

                            Ok(())
                        }
                    })
                }),
                Some(100),
                Some(400),
                Some(true),
            )
        };

        let mut credentials = None;
        if !matches!(app.visibility, flow_like::app::AppVisibility::Offline) {
            let token = self.token.as_ref().ok_or_else(|| {
                flow_like_types::anyhow!("No token registered, cannot run online event")
            })?;
            let hub_url = profile.hub_profile.hub.clone();
            if hub_url.is_empty() {
                return Err(flow_like_types::anyhow!(
                    "No hub URL configured, cannot get event credentials"
                ));
            }

            let shared_credentials =
                crate::execution_credentials::prepare(&hub_url, &self.app_id, Some(token), None)
                    .await?;
            credentials = Some(shared_credentials);
        }

        let mut renewable_state = (*execution_state).clone();
        if let Some(credentials) = &credentials {
            crate::execution_credentials::install_registry(&mut renewable_state, credentials)?;
        }
        let execution_state = Arc::new(renewable_state);

        let event_name = loaded_event.name.clone();
        let event_type = loaded_event.event_type.clone();

        let mut internal_run = InternalRun::from_template(
            &self.app_id,
            template,
            Some(loaded_event),
            &execution_state,
            &profile.hub_profile,
            &payload,
            false,
            buffered_sender.into_callback(),
            credentials,
            self.token.clone(),
            self.oauth_tokens.clone(),
            None,
            None,
        )
        .await?;

        internal_run
            .set_usage_attribution_from_visibility(&app.visibility)
            .await;

        internal_run.set_execution_environment(local_execution_environment());

        // Sink registrations authenticate with a PAT, which is not a JWT, so
        // the subject the run derived from it is the `local` placeholder.
        // Resolving against the hub recovers the PAT owner and their real role.
        if let Some(identity) = resolved_identity {
            internal_run.set_resolved_user_context(identity).await;
        } else {
            crate::execution_identity::apply_local_run_identity(
                &mut internal_run,
                &app.visibility,
                &self.app_id,
                self.token.as_deref(),
                &profile.hub_profile.hub,
                &flow_like_state,
            )
            .await;
        }

        let run_id = internal_run.run.lock().await.id.clone();

        let _send_result = buffered_sender
            .send(InterComEvent::with_type(
                "run_initiated",
                json::json!({ "run_id": run_id.clone()}),
            ))
            .await;

        let cancellation_token = CancellationToken::new();
        internal_run.set_cancellation_token(cancellation_token.clone());
        let board_name = internal_run.board.name.clone();
        let run_data = RunData::with_metadata(
            Some(self.app_id.clone()),
            &board_id,
            &payload.id,
            Some(self.event_id.clone()),
            cancellation_token.clone(),
            Some(board_name),
            Some(event_name),
            Some(event_type),
        );

        flow_like_state.register_run(&run_id, run_data);
        started.store(true, Ordering::Relaxed);

        let meta = tokio::select! {
            result = internal_run.execute(execution_state.clone()) => result,
            _ = cancellation_token.cancelled() => {
                println!("Board execution cancelled for run: {}", run_id);
                match tokio::time::timeout(Duration::from_secs(30), internal_run.flush_logs_cancelled()).await {
                    Ok(Ok(Some(meta))) => {
                        Some(meta)
                    },
                    Ok(Ok(None)) => {
                        println!("No meta flushing early");
                        None
                    },
                    Ok(Err(e)) => {
                        println!("Error flushing logs early for run: {}, {:?}", run_id, e);
                        None
                    },
                    Err(_) => {
                        println!("Timeout while flushing logs early for run: {}", run_id);
                        None
                    }
                }
            }
        };

        if let Err(err) = buffered_sender.flush().await {
            println!("Error flushing buffered sender: {}", err);
        }

        // Release the finished run from the registry; otherwise it stays
        // flagged "in use" and its logs can never be deleted from storage
        // management until restart.
        let _res = flow_like_state.remove_and_cancel_run(&run_id);

        Ok(meta)
    }
}

pub struct EventBus {
    sender: mpsc::Sender<EventBusEvent>,
    #[allow(dead_code)]
    // handle kept for future bus-side emits; every consumer currently passes its own AppHandle
    app_handle: AppHandle,
}

impl EventBus {
    pub fn new(app_handle: AppHandle) -> (Arc<Self>, mpsc::Receiver<EventBusEvent>) {
        let (sender, receiver) = mpsc::channel(MAX_QUEUE_SIZE);
        let new_self = Self { sender, app_handle };
        (Arc::new(new_self), receiver)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_event_with_token(
        &self,
        payload: Option<Value>,
        app_id: String,
        event_id: String,
        offline: bool,
        token: Option<String>,
        callback: Option<Arc<BufferedInterComHandler>>,
        oauth_tokens: Option<std::collections::HashMap<String, OAuthToken>>,
    ) -> Result<(), String> {
        if !offline && token.is_none() {
            return Err("No token registered, cannot send online events".to_string());
        }

        let event = EventBusEvent {
            payload,
            app_id,
            event_id,
            token,
            offline,
            callback,
            oauth_tokens: oauth_tokens.unwrap_or_default(),
        };

        self.sender
            .try_send(event)
            .map_err(|e| format!("Failed to send event: {}", e))
    }
}
