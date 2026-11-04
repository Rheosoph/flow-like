use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use super::TauriFunctionError;
use crate::state::{TauriFlowLikeState, TauriSettingsState};
use flow_like::{
    bit::{Bit, BitTypes},
    hub::Hub,
    models::stt::{LocalSttModel, LocalTranscriptionRequest},
};
use flow_like_types::{anyhow, tokio::sync::Mutex};
use serde::Serialize;

static MODELS: LazyLock<Mutex<HashMap<String, LocalSttModel>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Serialize)]
pub struct SpeechTranscription {
    pub text: String,
}

async fn resolve_bit(app_handle: &tauri::AppHandle, bit_id: &str) -> flow_like_types::Result<Bit> {
    let profile = TauriSettingsState::current_profile(app_handle).await?;
    let http_client = TauriFlowLikeState::http_client(app_handle).await?;
    let (hub, id) = bit_id
        .rsplit_once(':')
        .unwrap_or((&profile.hub_profile.hub, bit_id));
    let hub = Hub::new(hub, http_client).await?;
    let bit = hub.get_bit(id).await?;
    if bit.bit_type != BitTypes::Stt {
        return Err(anyhow!("Bit {} is not a speech-to-text model", bit_id));
    }
    Ok(bit)
}

async fn cached_model(
    app_handle: &tauri::AppHandle,
    bit_id: &str,
) -> flow_like_types::Result<LocalSttModel> {
    let bit = resolve_bit(app_handle, bit_id).await?;
    let key = LocalSttModel::cache_key_for(&bit)?;
    let mut models = MODELS.lock().await;
    if let Some(model) = models.get(&key) {
        return Ok(model.clone());
    }
    let state = TauriFlowLikeState::construct(app_handle).await?;
    let model = LocalSttModel::new(&bit, Arc::clone(&state)).await?;
    models.clear();
    models.insert(key, model.clone());
    Ok(model)
}

/// Transcribes one WAV utterance with an installed local STT bit; the loaded model stays cached
/// between calls, one model at a time.
#[tauri::command(async)]
pub async fn speech_transcribe(
    app_handle: tauri::AppHandle,
    bit_id: String,
    audio: Vec<u8>,
    language: Option<String>,
) -> Result<SpeechTranscription, TauriFunctionError> {
    let model = cached_model(&app_handle, &bit_id).await?;
    let output = model
        .transcribe(LocalTranscriptionRequest {
            audio_bytes: audio,
            file_name: "utterance.wav".to_string(),
            language,
            translate: false,
            timestamps: false,
        })
        .await?;
    Ok(SpeechTranscription {
        text: output.text.trim().to_string(),
    })
}
