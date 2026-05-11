use crate::{
    bit::{
        Bit, BitPack, BitTypes, TtsDTypePreference, TtsModelParameters, TtsModelType,
        TtsRuntimePreference,
    },
    flow::execution::context::ExecutionContext,
    models::local_utils::ensure_local_weights,
    state::FlowLikeState,
};
use any_tts::{
    AudioSamples, DType as AnyTtsDType, DeviceSelection, ModelAssetBundle,
    ModelInfo as AnyModelInfo, ModelType as AnyTtsModelType, ReferenceAudio, SynthesisRequest,
    TtsConfig, TtsModel,
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::{
    Cacheable, Result, anyhow,
    json::{Deserialize, Serialize},
    tokio,
};
use schemars::JsonSchema;
use std::{
    any::Any,
    collections::HashSet,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LocalTtsSynthesisRequest {
    pub text: String,
    pub language: Option<String>,
    pub voice: Option<String>,
    pub instruct: Option<String>,
    pub max_tokens: Option<usize>,
    pub temperature: Option<f64>,
    pub cfg_scale: Option<f64>,
    pub reference_audio: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LocalTtsModelInfo {
    pub name: String,
    pub variant: String,
    pub parameters: u64,
    pub sample_rate: u32,
    pub languages: Vec<String>,
    pub voices: Vec<String>,
}

impl From<AnyModelInfo> for LocalTtsModelInfo {
    fn from(value: AnyModelInfo) -> Self {
        Self {
            name: value.name,
            variant: value.variant,
            parameters: value.parameters,
            sample_rate: value.sample_rate,
            languages: value.languages,
            voices: value.voices,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TtsSynthesisOutput {
    pub wav: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_secs: f32,
    pub model_info: LocalTtsModelInfo,
}

#[derive(Clone)]
pub struct LocalTtsModel {
    pub bit: Arc<Bit>,
    pub cache_key: String,
    pub runtime: String,
    pub dtype: String,
    model: Arc<Mutex<Box<dyn TtsModel>>>,
}

impl Cacheable for LocalTtsModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl LocalTtsModel {
    pub fn cache_key_for(bit: &Bit) -> Result<String> {
        let params = bit
            .try_to_tts()
            .ok_or_else(|| anyhow!("Not a TTS model bit"))?;
        Ok(format!(
            "tts:{}:{}:{:?}:{:?}",
            bit.id, bit.dependency_tree_hash, params.runtime, params.dtype
        ))
    }

    pub async fn load_into_cache(context: &mut ExecutionContext, bit: &Bit) -> Result<String> {
        let cache_key = Self::cache_key_for(bit)?;
        if context.has_cache(&cache_key).await {
            return Ok(cache_key);
        }

        let model = Self::new(bit, context.app_state.clone()).await?;
        context.set_cache(&cache_key, Arc::new(model)).await;
        Ok(cache_key)
    }

    pub async fn from_cache(context: &mut ExecutionContext, cache_key: &str) -> Result<Self> {
        let cached = context
            .get_cache(cache_key)
            .await
            .ok_or_else(|| anyhow!("TTS model not found in cache"))?;
        let model = cached
            .as_any()
            .downcast_ref::<LocalTtsModel>()
            .ok_or_else(|| anyhow!("Failed to downcast cached TTS model"))?;
        Ok(model.clone())
    }

    pub async fn new(bit: &Bit, app_state: Arc<FlowLikeState>) -> Result<Self> {
        if bit.bit_type != BitTypes::Tts {
            return Err(anyhow!("Bit {} is not a TTS model", bit.id));
        }

        let params = bit
            .try_to_tts()
            .ok_or_else(|| anyhow!("Failed to parse TTS parameters"))?;

        let bit_store = FlowLikeState::bit_store(&app_state).await?;
        let bit_store = match bit_store {
            FlowLikeStore::Local(store) => store,
            _ => return Err(anyhow!("Local TTS requires the bits store to be local")),
        };

        let pack = bit.pack(app_state.clone()).await?;
        let resolved_assets = resolve_asset_bits(&params, bit, &pack)?;
        let asset_pack = BitPack {
            bits: deduplicate_bits(
                resolved_assets
                    .iter()
                    .map(|(_, asset_bit)| asset_bit.clone()),
            ),
        };
        ensure_local_weights(&asset_pack, &app_state, bit.id.as_str(), "TTS model").await?;

        let mut bundle = ModelAssetBundle::new();
        for (asset, asset_bit) in resolved_assets {
            let Some(path) = asset_bit.to_path(&bit_store) else {
                if asset.required {
                    return Err(anyhow!(
                        "TTS asset {} for bit {} has no local path",
                        asset.relative_path,
                        asset.bit
                    ));
                }
                continue;
            };

            let bytes = std::fs::read(&path).map_err(|error| {
                anyhow!(
                    "Failed to read TTS asset {} from {}: {}",
                    asset.relative_path,
                    path.display(),
                    error
                )
            })?;
            bundle.insert_bytes(asset.relative_path, bytes);
        }

        if bundle.is_empty() {
            return Err(anyhow!(
                "TTS bit {} has no local assets. Add dependency bits and map them in parameters.assets.",
                bit.id
            ));
        }

        let any_model_type = map_model_type(&params.model_type);
        let mut config = TtsConfig::new(any_model_type).with_asset_bundle(bundle);

        match params.runtime.clone().unwrap_or_default() {
            TtsRuntimePreference::Auto => {
                config = config.with_preferred_runtime();
            }
            runtime => {
                config = config.with_device(map_runtime(runtime)?);
            }
        }

        if let Some(dtype) = params.dtype.as_ref().and_then(map_dtype) {
            config = config.with_dtype(dtype);
        }

        let runtime = config.device.label();
        let dtype = config.dtype.label().to_string();
        let model = any_tts::load_model(config)
            .map_err(|error| anyhow!("Failed to load any-tts model {}: {}", bit.id, error))?;

        Ok(Self {
            bit: Arc::new(bit.clone()),
            cache_key: Self::cache_key_for(bit)?,
            runtime,
            dtype,
            model: Arc::new(Mutex::new(model)),
        })
    }

    pub async fn synthesize(
        &self,
        request: LocalTtsSynthesisRequest,
    ) -> Result<TtsSynthesisOutput> {
        let model = self.model.clone();
        tokio::task::spawn_blocking(move || {
            let synthesis_request = build_synthesis_request(request)?;
            let guard = model
                .lock()
                .map_err(|_| anyhow!("TTS model lock was poisoned"))?;
            let audio = guard
                .synthesize(&synthesis_request)
                .map_err(|error| anyhow!("TTS synthesis failed: {}", error))?;
            let model_info = guard.model_info().into();
            Ok(TtsSynthesisOutput {
                wav: audio.get_wav(),
                sample_rate: audio.sample_rate,
                channels: audio.channels,
                duration_secs: audio.duration_secs(),
                model_info,
            })
        })
        .await
        .map_err(|error| anyhow!("TTS synthesis task failed: {}", error))?
    }
}

fn build_synthesis_request(request: LocalTtsSynthesisRequest) -> Result<SynthesisRequest> {
    let mut synthesis_request = SynthesisRequest::new(request.text);

    if let Some(language) = clean_optional(request.language) {
        synthesis_request = synthesis_request.with_language(language);
    }
    if let Some(voice) = clean_optional(request.voice) {
        synthesis_request = synthesis_request.with_voice(voice);
    }
    if let Some(instruct) = clean_optional(request.instruct) {
        synthesis_request = synthesis_request.with_instruct(instruct);
    }
    if let Some(max_tokens) = request.max_tokens.filter(|value| *value > 0) {
        synthesis_request = synthesis_request.with_max_tokens(max_tokens);
    }
    if let Some(temperature) = request.temperature.filter(|value| *value > 0.0) {
        synthesis_request = synthesis_request.with_temperature(temperature);
    }
    if let Some(cfg_scale) = request.cfg_scale.filter(|value| *value > 0.0) {
        synthesis_request = synthesis_request.with_cfg_scale(cfg_scale);
    }
    if let Some(reference_audio) = request.reference_audio {
        let decoded = AudioSamples::from_audio_bytes(&reference_audio)
            .map_err(|error| anyhow!("Failed to decode reference audio: {}", error))?;
        synthesis_request = synthesis_request
            .with_reference_audio(ReferenceAudio::new(decoded.samples, decoded.sample_rate));
    }

    Ok(synthesis_request)
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != "auto")
}

fn resolve_asset_bits(
    params: &TtsModelParameters,
    parent_bit: &Bit,
    pack: &BitPack,
) -> Result<Vec<(crate::bit::TtsAssetRef, Bit)>> {
    if params.assets.is_empty() {
        return Err(anyhow!(
            "TTS bit {} has no asset map. Required files: {}",
            parent_bit.id,
            asset_requirements_label(&params.model_type)
        ));
    }

    let mut resolved = Vec::new();
    for asset in &params.assets {
        let matched = pack
            .bits
            .iter()
            .find(|bit| bit_matches_ref(bit, &asset.bit))
            .cloned();
        match matched {
            Some(bit) => resolved.push((asset.clone(), bit)),
            None if asset.required => {
                return Err(anyhow!(
                    "Required TTS asset bit {} for {} was not found in dependencies",
                    asset.bit,
                    asset.relative_path
                ));
            }
            None => {}
        }
    }

    Ok(resolved)
}

fn bit_matches_ref(bit: &Bit, reference: &str) -> bool {
    bit.id == reference || format!("{}:{}", bit.hub, bit.id) == reference
}

fn deduplicate_bits(bits: impl Iterator<Item = Bit>) -> Vec<Bit> {
    let mut seen = HashSet::new();
    bits.filter(|bit| seen.insert(format!("{}:{}", bit.hub, bit.id)))
        .collect()
}

fn map_model_type(model_type: &TtsModelType) -> AnyTtsModelType {
    match model_type {
        TtsModelType::Kokoro => AnyTtsModelType::Kokoro,
        TtsModelType::OmniVoice => AnyTtsModelType::OmniVoice,
        TtsModelType::Qwen3Tts => AnyTtsModelType::Qwen3Tts,
        TtsModelType::VibeVoice => AnyTtsModelType::VibeVoice,
        TtsModelType::VibeVoiceRealtime => AnyTtsModelType::VibeVoiceRealtime,
        TtsModelType::Voxtral => AnyTtsModelType::Voxtral,
    }
}

fn map_runtime(runtime: TtsRuntimePreference) -> Result<DeviceSelection> {
    match runtime {
        TtsRuntimePreference::Auto => Ok(DeviceSelection::Auto),
        TtsRuntimePreference::Cpu => Ok(DeviceSelection::Cpu),
        TtsRuntimePreference::Metal => metal_device(),
        TtsRuntimePreference::Cuda => cuda_device(),
        TtsRuntimePreference::Accelerate => accelerate_device(),
    }
}

fn metal_device() -> Result<DeviceSelection> {
    #[cfg(all(
        feature = "local-tts-metal",
        any(target_os = "macos", target_os = "ios")
    ))]
    {
        Ok(DeviceSelection::Metal(0))
    }
    #[cfg(not(all(
        feature = "local-tts-metal",
        any(target_os = "macos", target_os = "ios")
    )))]
    {
        Err(anyhow!(
            "Metal TTS runtime requires the local-tts-metal feature on an Apple platform"
        ))
    }
}

fn cuda_device() -> Result<DeviceSelection> {
    #[cfg(feature = "local-tts-cuda")]
    {
        Ok(DeviceSelection::Cuda(0))
    }
    #[cfg(not(feature = "local-tts-cuda"))]
    {
        Err(anyhow!(
            "CUDA TTS runtime requires the local-tts-cuda feature"
        ))
    }
}

fn accelerate_device() -> Result<DeviceSelection> {
    #[cfg(all(
        feature = "local-tts-accelerate",
        any(target_os = "macos", target_os = "ios")
    ))]
    {
        Ok(DeviceSelection::Cpu)
    }
    #[cfg(not(all(
        feature = "local-tts-accelerate",
        any(target_os = "macos", target_os = "ios")
    )))]
    {
        Err(anyhow!(
            "Accelerate TTS runtime requires the local-tts-accelerate feature on an Apple platform"
        ))
    }
}

fn map_dtype(dtype: &TtsDTypePreference) -> Option<AnyTtsDType> {
    match dtype {
        TtsDTypePreference::Auto => None,
        TtsDTypePreference::F32 => Some(AnyTtsDType::F32),
        TtsDTypePreference::F16 => Some(AnyTtsDType::F16),
        TtsDTypePreference::BF16 => Some(AnyTtsDType::BF16),
    }
}

fn asset_requirements_label(model_type: &TtsModelType) -> String {
    let any_model_type = map_model_type(model_type);
    any_model_type
        .asset_requirements()
        .iter()
        .map(|requirement| {
            let required = if requirement.required {
                "required"
            } else {
                "optional"
            };
            format!("{} ({})", requirement.pattern, required)
        })
        .collect::<Vec<_>>()
        .join(", ")
}
