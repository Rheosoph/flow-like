use std::sync::Arc;

use flow_like::{
    app::{App, AppVisibility},
    bit::{Bit, BitModelPreference},
    flow_like_model_provider::{
        history::{History, HistoryMessage, HistoryThinking},
        llm::LLMCallback,
        response::Response,
    },
    flow_like_types::intercom::{BufferedInterComHandler, InterComEvent},
    models::{device::Interaction, llm::ModelUsageContext},
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

fn inline_completion_preferences() -> BitModelPreference {
    BitModelPreference {
        cost_weight: Some(1.0),
        ..BitModelPreference::default()
    }
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

    let best_model = if remote {
        current_profile
            .hub_profile
            .get_best_model(&preferences, multimodal, true, http_client)
            .await?
    } else {
        let state = TauriFlowLikeState::construct(&app_handle).await?;
        let capabilities =
            flow_like::state::FlowLikeState::completion_model_capabilities(&state).await;
        let devices = state.device_model_probe(Interaction::Forbidden);
        current_profile
            .hub_profile
            .resolve_completion_model(
                None,
                &preferences,
                multimodal,
                capabilities,
                devices.as_ref(),
                http_client,
            )
            .await?
    };

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

    // Inline suggestions rank the active profile's models by cost efficiency.
    let preferences = inline_completion_preferences();
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let capabilities =
        flow_like::state::FlowLikeState::completion_model_capabilities(&flow_like_state).await;
    // Typing must not open a device connection prompt.
    let devices = flow_like_state.device_model_probe(Interaction::Forbidden);

    let best_model = current_profile
        .hub_profile
        .resolve_completion_model(
            None,
            &preferences,
            false,
            capabilities,
            devices.as_ref(),
            http_client,
        )
        .await?;

    let model = {
        let usage_context =
            resolve_model_usage_context(app_id.as_deref(), flow_like_state.clone()).await?;
        let model_factory = flow_like_state.model_factory.clone();

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
    history.max_completion_tokens = Some(256);
    history.thinking = Some(HistoryThinking::Off);
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
    let devices = flow_like_state.device_model_probe(Interaction::Allowed { run_label: None });

    let best_model = current_profile
        .hub_profile
        .resolve_completion_model(
            None,
            &preferences,
            false,
            capabilities,
            devices.as_ref(),
            http_client,
        )
        .await?;

    let model = {
        let usage_context =
            resolve_model_usage_context(app_id.as_deref(), flow_like_state.clone()).await?;
        let model_factory = flow_like_state.model_factory.clone();

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
    use flow_like::{
        bit::BitTypes,
        profile::{Profile, ProfileCustomBit},
        state::CompletionModelCapabilities,
        utils::http::HTTPClient,
    };

    fn completion_bit(id: &str, provider: &str, cost_efficiency: f32) -> Bit {
        Bit {
            id: id.to_string(),
            bit_type: BitTypes::Llm,
            parameters: flow_like_types::json::json!({
                "context_length": 20_000,
                "model_classification": {
                    "cost": cost_efficiency,
                    "speed": 0.5,
                    "reasoning": 0.5,
                    "creativity": 0.5,
                    "factuality": 0.5,
                    "function_calling": 0.5,
                    "safety": 0.5,
                    "openness": 0.5,
                    "multilinguality": 0.5,
                    "coding": 0.5,
                },
                "provider": { "provider_name": provider, "model_id": id },
            }),
            ..Bit::default()
        }
    }

    #[tokio::test]
    async fn inline_completion_selects_the_cheapest_available_active_profile_bit() {
        let profile = Profile {
            bits: vec!["expensive".into(), "cheap".into(), "local".into()],
            custom_bits: vec![
                ProfileCustomBit(completion_bit("expensive", "hosted:openai", 0.2)),
                ProfileCustomBit(completion_bit("cheap", "hosted:openai", 0.8)),
                ProfileCustomBit(completion_bit("local", "Local", 0.9)),
                ProfileCustomBit(completion_bit("inactive", "hosted:openai", 1.0)),
            ],
            ..Profile::default()
        };
        let selected = profile
            .resolve_completion_model(
                None,
                &inline_completion_preferences(),
                false,
                CompletionModelCapabilities::default(),
                None,
                Arc::new(HTTPClient::new_without_refetch()),
            )
            .await
            .unwrap();

        assert_eq!(selected.id, "cheap");
    }

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
