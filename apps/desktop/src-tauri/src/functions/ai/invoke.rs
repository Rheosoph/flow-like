use std::sync::Arc;

use flow_like::{
    app::{App, AppVisibility},
    bit::{Bit, BitModelPreference},
    flow_like_model_provider::{
        history::{History, HistoryMessage},
        llm::LLMCallback,
        response::Response,
    },
    flow_like_types::intercom::{BufferedInterComHandler, InterComEvent},
    models::llm::ModelUsageContext,
};
use tauri::{AppHandle, ipc::Channel};

use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};

fn model_usage_app_id(app_id: &str, visibility: &AppVisibility) -> Option<String> {
    if matches!(visibility, AppVisibility::Offline) {
        None
    } else {
        Some(app_id.to_string())
    }
}

async fn resolve_model_usage_context(
    app_id: Option<&str>,
    flow_like_state: Arc<flow_like::state::FlowLikeState>,
) -> Result<Option<ModelUsageContext>, TauriFunctionError> {
    let Some(app_id) = app_id.map(str::trim).filter(|app_id| !app_id.is_empty()) else {
        return Ok(None);
    };

    let app = App::load(app_id.to_string(), flow_like_state).await?;
    Ok(Some(ModelUsageContext {
        app_id: model_usage_app_id(app_id, &app.visibility),
        run_id: None,
        api_base_url: None,
    }))
}

#[tauri::command(async)]
pub async fn find_best_model(
    app_handle: AppHandle,
    preferences: BitModelPreference,
    multimodal: bool,
    remote: bool,
) -> Result<Bit, TauriFunctionError> {
    let current_profile = TauriSettingsState::current_profile(&app_handle).await?;
    let http_client = TauriFlowLikeState::http_client(&app_handle).await?;

    let best_model = current_profile
        .hub_profile
        .get_best_model(&preferences, multimodal, remote, http_client)
        .await?;

    Ok(best_model)
}

#[tauri::command(async)]
pub async fn chat_completion(
    app_handle: AppHandle,
    messages: Vec<HistoryMessage>,
    token: Option<String>,
    app_id: Option<String>,
) -> Result<Response, TauriFunctionError> {
    let current_profile = TauriSettingsState::current_profile(&app_handle).await?;
    let http_client = TauriFlowLikeState::http_client(&app_handle).await?;

    let preferences = BitModelPreference::default();
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let capabilities =
        flow_like::state::FlowLikeState::completion_model_capabilities(&flow_like_state).await;

    let best_model = current_profile
        .hub_profile
        .resolve_completion_model(None, &preferences, false, capabilities, http_client)
        .await?;

    let model = {
        let usage_context =
            resolve_model_usage_context(app_id.as_deref(), flow_like_state.clone()).await?;
        let model_factory = flow_like_state.model_factory.clone();
        let mut model_factory = model_factory.lock().await;

        match model_factory
            .build(&best_model, flow_like_state, token, usage_context)
            .await
        {
            Ok(model) => model,
            Err(e) => {
                return Err(TauriFunctionError::new(&format!(
                    "Error building model: {}",
                    e
                )));
            }
        }
    };

    let callback: LLMCallback = Arc::new(move |_response| Box::pin(async move { Ok(()) }));

    let mut history = History::new("local".to_string(), vec![]);
    history.messages.extend(messages);
    history.set_stream(false);
    let res = model.invoke(&history, Some(callback)).await?;

    Ok(res)
}

#[tauri::command(async)]
pub async fn stream_chat_completion(
    app_handle: AppHandle,
    messages: Vec<HistoryMessage>,
    on_chunk: Channel<Vec<InterComEvent>>,
    token: Option<String>,
    app_id: Option<String>,
) -> Result<Response, TauriFunctionError> {
    let current_profile = TauriSettingsState::current_profile(&app_handle).await?;
    let http_client = TauriFlowLikeState::http_client(&app_handle).await?;

    let preferences = BitModelPreference::default();
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let capabilities =
        flow_like::state::FlowLikeState::completion_model_capabilities(&flow_like_state).await;

    let best_model = current_profile
        .hub_profile
        .resolve_completion_model(None, &preferences, false, capabilities, http_client)
        .await?;

    let model = {
        let usage_context =
            resolve_model_usage_context(app_id.as_deref(), flow_like_state.clone()).await?;
        let model_factory = flow_like_state.model_factory.clone();
        let mut model_factory = model_factory.lock().await;

        match model_factory
            .build(&best_model, flow_like_state, token, usage_context)
            .await
        {
            Ok(model) => model,
            Err(e) => {
                return Err(TauriFunctionError::new(&format!(
                    "Error building model: {}",
                    e
                )));
            }
        }
    };

    let buffered_sender = Arc::new(BufferedInterComHandler::new(
        Arc::new(move |chunks| {
            let on_chunk = on_chunk.clone();
            Box::pin(async move {
                if let Err(err) = on_chunk.send(chunks) {
                    println!("Error sending chunk: {}", err);
                };
                Ok(())
            })
        }),
        Some(20),
        Some(100),
        Some(true),
    ));

    let finalized = buffered_sender.clone();

    let callback: LLMCallback = Arc::new(move |response| {
        Box::pin({
            let buffered_handler = buffered_sender.clone();
            async move {
                let event = InterComEvent::with_type("chunk".to_string(), response.clone());
                buffered_handler.send(event).await?;
                Ok(())
            }
        })
    });

    let mut history = History::new("local".to_string(), vec![]);
    history.messages.extend(messages);
    history.set_stream(true);
    let res = model.invoke(&history, Some(callback)).await?;
    finalized.flush().await?;

    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_editor_usage_is_not_app_attributed() {
        assert_eq!(
            model_usage_app_id("local-app", &AppVisibility::Offline),
            None
        );
    }

    #[test]
    fn online_editor_usage_is_app_attributed() {
        assert_eq!(
            model_usage_app_id("online-app", &AppVisibility::Private),
            Some("online-app".to_string())
        );
    }
}
