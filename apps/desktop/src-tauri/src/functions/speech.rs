use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use super::TauriFunctionError;
use crate::state::{TauriFlowLikeState, TauriSettingsState};
use flow_like::{
    bit::{Bit, BitTypes, TtsModelParameters},
    hub::Hub,
    models::stt::{LocalSttModel, LocalTranscriptionRequest},
    models::tts::{LocalTtsModel, LocalTtsSynthesisRequest},
};
use flow_like_types::{anyhow, tokio::sync::Mutex};
use serde::Serialize;

static MODELS: LazyLock<Mutex<HashMap<String, LocalSttModel>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static TTS_MODELS: LazyLock<Mutex<HashMap<String, LocalTtsModel>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Serialize)]
pub struct SpeechTranscription {
    pub text: String,
}

async fn resolve_bit(
    app_handle: &tauri::AppHandle,
    bit_id: &str,
    expected: BitTypes,
) -> flow_like_types::Result<Bit> {
    let profile = TauriSettingsState::current_profile(app_handle).await?;
    let http_client = TauriFlowLikeState::http_client(app_handle).await?;
    let (hub, id) = bit_id
        .rsplit_once(':')
        .unwrap_or((&profile.hub_profile.hub, bit_id));
    let hub = Hub::new(hub, http_client).await?;
    let bit = hub.get_bit(id).await?;
    if bit.bit_type != expected {
        return Err(anyhow!("Bit {} is not a {:?} model", bit_id, expected));
    }
    Ok(bit)
}

async fn cached_model(
    app_handle: &tauri::AppHandle,
    bit_id: &str,
) -> flow_like_types::Result<LocalSttModel> {
    let bit = resolve_bit(app_handle, bit_id, BitTypes::Stt).await?;
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

fn local_tts_parameters(bit: &Bit) -> flow_like_types::Result<TtsModelParameters> {
    let params = bit
        .try_to_tts()
        .ok_or_else(|| anyhow!("Bit {} is not a local TTS model", bit.id))?;
    if !params
        .provider
        .provider_name
        .trim()
        .eq_ignore_ascii_case("local:any-tts")
    {
        return Err(anyhow!("Bit {} is not a local TTS model", bit.id));
    }
    Ok(params)
}

fn synthesis_request(
    bit: &Bit,
    text: String,
    voice: Option<String>,
) -> flow_like_types::Result<LocalTtsSynthesisRequest> {
    validate_synthesis_text(&text)?;
    let params = local_tts_parameters(bit)?;
    Ok(LocalTtsSynthesisRequest {
        text,
        language: params.default_language,
        voice: voice
            .filter(|voice| !voice.trim().is_empty())
            .or(params.default_voice),
        instruct: None,
        max_tokens: None,
        temperature: None,
        cfg_scale: None,
        reference_audio: None,
    })
}

fn validate_synthesis_text(text: &str) -> flow_like_types::Result<()> {
    if text.trim().is_empty()
        || text.chars().count() > flow_like::flow_like_model_provider::audio::MAX_SPEECH_CHARS
    {
        return Err(anyhow!(
            "Speech text must contain between 1 and 4096 characters"
        ));
    }
    Ok(())
}

async fn cached_tts_model(
    app_handle: &tauri::AppHandle,
    bit_id: &str,
) -> flow_like_types::Result<LocalTtsModel> {
    let bit = resolve_bit(app_handle, bit_id, BitTypes::Tts).await?;
    local_tts_parameters(&bit)?;
    let key = LocalTtsModel::cache_key_for(&bit)?;
    let mut models = TTS_MODELS.lock().await;
    if let Some(model) = models.get(&key) {
        let mut model = model.clone();
        model.bit = Arc::new(bit);
        return Ok(model);
    }
    let state = TauriFlowLikeState::construct(app_handle).await?;
    let model = LocalTtsModel::new(&bit, state).await?;
    models.clear();
    models.insert(key, model.clone());
    Ok(model)
}

/// Synthesizes one reply with a local TTS bit and returns WAV bytes. Reuses the loaded model
/// between replies, keeping at most one speech model in this command's cache.
#[tauri::command(async)]
pub async fn speech_synthesize(
    app_handle: tauri::AppHandle,
    bit_id: String,
    text: String,
    voice: Option<String>,
) -> Result<tauri::ipc::Response, TauriFunctionError> {
    validate_synthesis_text(&text)?;
    let model = cached_tts_model(&app_handle, &bit_id).await?;
    let request = synthesis_request(&model.bit, text, voice)?;
    let output = model.synthesize(request).await?;
    Ok(tauri::ipc::Response::new(output.wav))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tts_bit() -> Bit {
        Bit {
            id: "local-voice".into(),
            bit_type: BitTypes::Tts,
            parameters: json!({
                "provider": {"provider_name": "local:any-tts"},
                "default_language": "en", "default_voice": "af_heart",
            }),
            ..Default::default()
        }
    }

    #[test]
    fn local_speech_uses_model_defaults_and_explicit_voice() {
        let bit = tts_bit();
        let request = synthesis_request(&bit, "Hello".into(), None).unwrap();
        assert_eq!(request.language.as_deref(), Some("en"));
        assert_eq!(request.voice.as_deref(), Some("af_heart"));
        let request = synthesis_request(&bit, "Hello".into(), Some("af_bella".into())).unwrap();
        assert_eq!(request.voice.as_deref(), Some("af_bella"));
    }

    #[test]
    fn local_speech_rejects_hosted_bits_and_invalid_text() {
        let mut bit = tts_bit();
        bit.parameters["provider"]["provider_name"] = json!("hosted:openrouter");
        assert!(synthesis_request(&bit, "Hello".into(), None).is_err());
        let mut bit = tts_bit();
        bit.bit_type = BitTypes::Stt;
        assert!(synthesis_request(&bit, "Hello".into(), None).is_err());
        for text in [" ".into(), "x".repeat(4097)] {
            assert!(synthesis_request(&tts_bit(), text, None).is_err());
        }
        assert!(synthesis_request(&tts_bit(), "é".repeat(4096), None).is_ok());
    }
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
