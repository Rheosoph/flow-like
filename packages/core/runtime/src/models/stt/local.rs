use crate::{
    bit::{
        Bit, BitPack, BitTypes, SttAssetRef, SttDTypePreference, SttModelParameters, SttModelType,
        SttRuntimePreference,
    },
    flow::execution::context::ExecutionContext,
    models::local_utils::ensure_local_weights,
    state::FlowLikeState,
};
use any_speech_to_text::{
    AudioInput, DType as AnySttDType, DeviceSelection, ModelAssetBundle, ModelInfo as AnyModelInfo,
    ModelType as AnySttModelType, SttConfig, SttModel, TranscriptionRequest, TranscriptionResult,
    TranscriptionSegment, TranscriptionTask, load_model,
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
pub struct LocalTranscriptionRequest {
    pub audio_bytes: Vec<u8>,
    pub file_name: String,
    pub language: Option<String>,
    pub translate: bool,
    pub timestamps: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LocalSttModelInfo {
    pub backend: String,
    pub model_type: String,
    pub display_name: String,
    pub sample_rate: u32,
    pub supported_languages: Vec<String>,
}

impl From<AnyModelInfo> for LocalSttModelInfo {
    fn from(value: AnyModelInfo) -> Self {
        Self {
            backend: value.backend.label().to_string(),
            model_type: value.model_type.label().to_string(),
            display_name: value.display_name,
            sample_rate: value.sample_rate,
            supported_languages: value.supported_languages,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TranscriptionSegmentOutput {
    pub start_s: f32,
    pub end_s: f32,
    pub speaker: Option<String>,
    pub text: String,
}

impl From<TranscriptionSegment> for TranscriptionSegmentOutput {
    fn from(value: TranscriptionSegment) -> Self {
        Self {
            start_s: value.start_s,
            end_s: value.end_s,
            speaker: value.speaker,
            text: value.text,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TranscriptionOutput {
    pub text: String,
    pub language: Option<String>,
    pub segments: Vec<TranscriptionSegmentOutput>,
    pub duration_secs: f32,
    pub model_info: LocalSttModelInfo,
}

#[derive(Clone)]
pub struct LocalSttModel {
    pub bit: Arc<Bit>,
    pub cache_key: String,
    pub runtime: String,
    pub dtype: String,
    model: Arc<Mutex<Box<dyn SttModel>>>,
}

impl Cacheable for LocalSttModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl LocalSttModel {
    pub fn cache_key_for(bit: &Bit) -> Result<String> {
        let params = bit
            .try_to_stt()
            .ok_or_else(|| anyhow!("Not a local STT model bit"))?;
        Ok(format!(
            "stt:{}:{}:{:?}:{:?}",
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
            .ok_or_else(|| anyhow!("STT model not found in cache"))?;
        let model = cached
            .as_any()
            .downcast_ref::<LocalSttModel>()
            .ok_or_else(|| anyhow!("Failed to downcast cached STT model"))?;
        Ok(model.clone())
    }

    pub async fn new(bit: &Bit, app_state: Arc<FlowLikeState>) -> Result<Self> {
        if bit.bit_type != BitTypes::Stt {
            return Err(anyhow!("Bit {} is not an STT model", bit.id));
        }

        let params = bit
            .try_to_stt()
            .ok_or_else(|| anyhow!("Failed to parse local STT parameters"))?;

        let bit_store = FlowLikeState::bit_store(&app_state).await?;
        let bit_store = match bit_store {
            FlowLikeStore::Local(store) => store,
            _ => return Err(anyhow!("Local STT requires the bits store to be local")),
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
        ensure_local_weights(&asset_pack, &app_state, bit.id.as_str(), "STT model").await?;

        let mut bundle = ModelAssetBundle::new();
        for (asset, asset_bit) in resolved_assets {
            let Some(path) = asset_bit.to_path(&bit_store) else {
                if asset.required {
                    return Err(anyhow!(
                        "STT asset {} for bit {} has no local path",
                        asset.relative_path,
                        asset.bit
                    ));
                }
                continue;
            };

            let bytes = tokio::fs::read(&path).await.map_err(|error| {
                anyhow!(
                    "Failed to read STT asset {} from {}: {}",
                    asset.relative_path,
                    path.display(),
                    error
                )
            })?;
            bundle.insert_bytes(asset.relative_path, bytes);
        }

        if bundle.is_empty() {
            return Err(anyhow!(
                "STT bit {} has no local assets. Add dependency bits and map them in parameters.assets.",
                bit.id
            ));
        }

        let any_model_type = map_model_type(&params.model_type);
        let mut config = SttConfig::new(any_model_type).with_asset_bundle(bundle);

        match params.runtime.clone().unwrap_or_default() {
            SttRuntimePreference::Auto => {
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
        let model = load_model(config).map_err(|error| {
            anyhow!(
                "Failed to load any-speech-to-text model {}: {}",
                bit.id,
                error
            )
        })?;

        Ok(Self {
            bit: Arc::new(bit.clone()),
            cache_key: Self::cache_key_for(bit)?,
            runtime,
            dtype,
            model: Arc::new(Mutex::new(model)),
        })
    }

    pub async fn transcribe(
        &self,
        request: LocalTranscriptionRequest,
    ) -> Result<TranscriptionOutput> {
        let model = self.model.clone();
        tokio::task::spawn_blocking(move || {
            let transcription_request = build_transcription_request(request);
            let guard = model
                .lock()
                .map_err(|_| anyhow!("STT model lock was poisoned"))?;
            let result: TranscriptionResult = guard
                .transcribe(&transcription_request)
                .map_err(|error| anyhow!("STT transcription failed: {}", error))?;
            let model_info = guard.model_info().clone().into();
            Ok(TranscriptionOutput {
                text: result.text,
                language: result.language,
                segments: result
                    .segments
                    .into_iter()
                    .map(TranscriptionSegmentOutput::from)
                    .collect(),
                duration_secs: result.duration_s,
                model_info,
            })
        })
        .await
        .map_err(|error| anyhow!("STT transcription task failed: {}", error))?
    }
}

fn build_transcription_request(request: LocalTranscriptionRequest) -> TranscriptionRequest {
    let input = AudioInput::from_bytes(request.file_name, request.audio_bytes);
    let mut transcription_request = TranscriptionRequest::new(input);

    if let Some(language) = clean_language(request.language) {
        transcription_request = transcription_request.with_language(language);
    }
    if request.translate {
        transcription_request = transcription_request.with_task(TranscriptionTask::Translate);
    }
    transcription_request = transcription_request.with_timestamps(request.timestamps);

    transcription_request
}

fn clean_language(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && !is_default_option(value))
}

fn is_default_option(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "auto" | "default" | "none" | "null"
    )
}

fn resolve_asset_bits(
    params: &SttModelParameters,
    parent_bit: &Bit,
    pack: &BitPack,
) -> Result<Vec<(SttAssetRef, Bit)>> {
    if params.assets.is_empty() {
        return Err(anyhow!(
            "STT bit {} has no asset map. Required files: {}",
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
                    "Required STT asset bit {} for {} was not found in dependencies",
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

fn map_model_type(model_type: &SttModelType) -> AnySttModelType {
    match model_type {
        SttModelType::WhisperTiny => AnySttModelType::WhisperTiny,
        SttModelType::WhisperTinyEn => AnySttModelType::WhisperTinyEn,
        SttModelType::WhisperBase => AnySttModelType::WhisperBase,
        SttModelType::WhisperBaseEn => AnySttModelType::WhisperBaseEn,
        SttModelType::WhisperSmall => AnySttModelType::WhisperSmall,
        SttModelType::WhisperSmallEn => AnySttModelType::WhisperSmallEn,
        SttModelType::WhisperMedium => AnySttModelType::WhisperMedium,
        SttModelType::WhisperMediumEn => AnySttModelType::WhisperMediumEn,
        SttModelType::WhisperLargeV3 => AnySttModelType::WhisperLargeV3,
        SttModelType::WhisperLargeV3Turbo => AnySttModelType::WhisperLargeV3Turbo,
        SttModelType::DistilWhisperMediumEn => AnySttModelType::DistilWhisperMediumEn,
        SttModelType::DistilWhisperLargeV2 => AnySttModelType::DistilWhisperLargeV2,
        SttModelType::DistilWhisperLargeV3 => AnySttModelType::DistilWhisperLargeV3,
        SttModelType::OlmoAsrTinyEn => AnySttModelType::OlmoAsrTinyEn,
        SttModelType::OlmoAsrBaseEn => AnySttModelType::OlmoAsrBaseEn,
        SttModelType::OlmoAsrSmallEn => AnySttModelType::OlmoAsrSmallEn,
        SttModelType::OlmoAsrMediumEn => AnySttModelType::OlmoAsrMediumEn,
        SttModelType::OlmoAsrLargeEn => AnySttModelType::OlmoAsrLargeEn,
        SttModelType::OlmoAsrLargeEnV2 => AnySttModelType::OlmoAsrLargeEnV2,
        SttModelType::Qwen3Asr17B => AnySttModelType::Qwen3Asr17B,
        SttModelType::MoonshineBaseEn => AnySttModelType::MoonshineBaseEn,
    }
}

fn map_runtime(runtime: SttRuntimePreference) -> Result<DeviceSelection> {
    match runtime {
        SttRuntimePreference::Auto => Ok(DeviceSelection::Auto),
        SttRuntimePreference::Cpu => Ok(DeviceSelection::Cpu),
        SttRuntimePreference::Metal => metal_device(),
        SttRuntimePreference::Cuda => cuda_device(),
        SttRuntimePreference::Accelerate => accelerate_device(),
    }
}

fn metal_device() -> Result<DeviceSelection> {
    #[cfg(all(
        feature = "local-stt-metal",
        any(target_os = "macos", target_os = "ios")
    ))]
    {
        Ok(DeviceSelection::Metal(0))
    }
    #[cfg(not(all(
        feature = "local-stt-metal",
        any(target_os = "macos", target_os = "ios")
    )))]
    {
        Err(anyhow!(
            "Metal STT runtime requires the local-stt-metal feature on an Apple platform"
        ))
    }
}

fn cuda_device() -> Result<DeviceSelection> {
    #[cfg(feature = "local-stt-cuda")]
    {
        Ok(DeviceSelection::Cuda(0))
    }
    #[cfg(not(feature = "local-stt-cuda"))]
    {
        Err(anyhow!(
            "CUDA STT runtime requires the local-stt-cuda feature"
        ))
    }
}

fn accelerate_device() -> Result<DeviceSelection> {
    #[cfg(all(
        feature = "local-stt-accelerate",
        any(target_os = "macos", target_os = "ios")
    ))]
    {
        Ok(DeviceSelection::Cpu)
    }
    #[cfg(not(all(
        feature = "local-stt-accelerate",
        any(target_os = "macos", target_os = "ios")
    )))]
    {
        Err(anyhow!(
            "Accelerate STT runtime requires the local-stt-accelerate feature on an Apple platform"
        ))
    }
}

fn map_dtype(dtype: &SttDTypePreference) -> Option<AnySttDType> {
    match dtype {
        SttDTypePreference::Auto => None,
        SttDTypePreference::F32 => Some(AnySttDType::F32),
        SttDTypePreference::F16 => Some(AnySttDType::F16),
        SttDTypePreference::BF16 => Some(AnySttDType::BF16),
    }
}

fn asset_requirements_label(model_type: &SttModelType) -> String {
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
