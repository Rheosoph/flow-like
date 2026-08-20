use crate::{
    state::TauriSettingsState,
    utils::{UiEmitTarget, local_execution_environment},
};
use flow_like::app::App;
use flow_like::flow::execution::{InternalRun, LogMeta};
use flow_like::flow::oauth::OAuthToken;
use flow_like::flow_like_storage::Path;
use flow_like::hub::Hub;
use flow_like::state::RunData;
use flow_like::{flow::execution::RunPayload, state::FlowLikeState};
use flow_like_types::intercom::{BufferedInterComHandler, InterComEvent};
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::{Value, sync::mpsc};
use flow_like_types::{json, tokio};
use std::sync::Arc;
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
        let execution_state = Arc::new(flow_like_state.for_execution_run());

        let Ok(app) = App::load(self.app_id.clone(), execution_state.clone()).await else {
            return Err(flow_like_types::anyhow!("App not found"));
        };

        let loaded_event = app.get_event(&self.event_id, None).await?;
        let payload = RunPayload {
            id: loaded_event.node_id.clone(),
            payload: self.payload.to_owned(),
            runtime_variables: None, // Event bus triggers don't have runtime vars context
            filter_secrets: Some(false), // Desktop execution is trusted
        };

        let board_version = loaded_event.board_version;
        let board_id = loaded_event.board_id.clone();

        let Ok(board) = app.open_board(board_id.clone(), None, board_version).await else {
            return Err(flow_like_types::anyhow!("Board not found"));
        };

        let board = Arc::new(board.lock().await.clone());
        let profile = TauriSettingsState::current_profile(app_handle).await?;

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
                                crate::utils::emit_throttled(
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
        if !self.offline {
            let token = self.token.as_ref().ok_or_else(|| {
                flow_like_types::anyhow!("No token registered, cannot run online event")
            })?;
            let hub_url = profile.hub_profile.hub.clone();
            if hub_url.is_empty() {
                return Err(flow_like_types::anyhow!(
                    "No hub URL configured, cannot get event credentials"
                ));
            }

            let hub = Hub::new(&hub_url, flow_like_state.http_client.clone()).await?;
            let shared_credentials = hub.shared_credentials(token, &self.app_id).await?;
            credentials = Some(shared_credentials);
        }

        let event_name = loaded_event.name.clone();
        let event_type = loaded_event.event_type.clone();

        let mut internal_run = InternalRun::new(
            &self.app_id,
            board,
            Some(loaded_event),
            &execution_state,
            &profile.hub_profile,
            &payload,
            false,
            buffered_sender.into_callback(),
            credentials,
            self.token.clone(),
            self.oauth_tokens.clone(),
        )
        .await?;

        internal_run
            .set_usage_attribution_from_visibility(&app.visibility)
            .await;

        internal_run.set_execution_environment(local_execution_environment());

        // Sink registrations authenticate with a PAT, which is not a JWT, so
        // the subject the run derived from it is the `local` placeholder.
        // Resolving against the hub recovers the PAT owner and their real role.
        crate::execution_identity::apply_local_run_identity(
            &mut internal_run,
            &app.visibility,
            &self.app_id,
            self.token.as_deref(),
            &profile.hub_profile.hub,
            &flow_like_state,
        )
        .await;

        let run_id = internal_run.run.lock().await.id.clone();

        let _send_result = buffered_sender
            .send(InterComEvent::with_type(
                "run_initiated",
                json::json!({ "run_id": run_id.clone()}),
            ))
            .await;

        let cancellation_token = CancellationToken::new();
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

        let app_id = self.app_id.clone();

        if let Err(err) = buffered_sender.flush().await {
            println!("Error flushing buffered sender: {}", err);
        }

        let flush_result: flow_like_types::Result<()> = if let Some(meta) = &meta {
            let (db_fn, write_options) = {
                let guard = execution_state.config.read().await;
                (
                    guard.callbacks.build_logs_database.clone(),
                    guard.callbacks.lance_write_options.clone(),
                )
            };
            async {
                let db_fn = db_fn
                    .as_ref()
                    .ok_or_else(|| flow_like_types::anyhow!("No log database configured"))?;
                let base_path = Path::from("runs").child(app_id).child(board_id);
                let db = execution_state
                    .with_lance_session(db_fn(base_path.clone()))
                    .execute()
                    .await
                    .map_err(|e| {
                        flow_like_types::anyhow!("Failed to open database: {}, {:?}", base_path, e)
                    })?;
                meta.flush(db, write_options.as_ref()).await.map_err(|e| {
                    flow_like_types::anyhow!("Failed to flush run: {}, {:?}", base_path, e)
                })?;
                Ok(())
            }
            .await
        } else {
            Ok(())
        };

        // Always release the finished run from the registry, even if flushing
        // its logs failed. Otherwise the run stays flagged "in use" and its
        // logs can never be deleted from storage management until restart.
        let _res = flow_like_state.remove_and_cancel_run(&run_id);
        flush_result?;

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
