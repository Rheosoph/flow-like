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
    TtsConfig, TtsError, TtsModel,
};
use flow_like_model_provider::text_splitter::{ChunkConfig, MarkdownSplitter, TextSplitter};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::{
    Cacheable, Error, Result, anyhow,
    json::{Deserialize, Serialize},
    tokio,
};
use schemars::JsonSchema;
use std::{
    any::Any,
    collections::HashSet,
    sync::{Arc, Mutex},
};

const KOKORO_TEXT_CHUNK_CHARS: usize = 350;
const KOKORO_CHUNK_PAUSE_SECS: f32 = 0.18;

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
        let model_type = self
            .bit
            .try_to_tts()
            .map(|params| params.model_type)
            .unwrap_or_default();
        tokio::task::spawn_blocking(move || {
            let synthesis_request = build_synthesis_request(request)?;
            let guard = model
                .lock()
                .map_err(|_| anyhow!("TTS model lock was poisoned"))?;
            let audio = synthesize_with_model(guard.as_ref(), &synthesis_request, &model_type)?;
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

fn synthesize_with_model(
    model: &dyn TtsModel,
    request: &SynthesisRequest,
    model_type: &TtsModelType,
) -> Result<AudioSamples> {
    let request = resolve_synthesis_options(model, request)?;

    if matches!(model_type, TtsModelType::Kokoro) {
        return synthesize_kokoro_chunked(model, &request);
    }

    model
        .synthesize(&request)
        .map_err(|error| tts_synthesis_error(model, &request, error))
}

fn resolve_synthesis_options(
    model: &dyn TtsModel,
    request: &SynthesisRequest,
) -> Result<SynthesisRequest> {
    if request.language.is_none() && request.voice.is_none() && request.reference_audio.is_none() {
        return Ok(request.clone());
    }

    let model_info = model.model_info();
    let mut resolved = request.clone();

    if let Some(language) = request.language.as_deref() {
        resolved.language = Some(resolve_language_for_model(language, &model_info)?);
    }

    if request.reference_audio.is_none()
        && let Some(voice) = request.voice.as_deref() {
            resolved.voice = Some(resolve_voice_for_model(voice, &model_info)?);
        }

    Ok(resolved)
}

fn resolve_language_for_model(language: &str, model_info: &AnyModelInfo) -> Result<String> {
    if model_info.languages.is_empty()
        || model_info
            .languages
            .iter()
            .all(|value| is_permissive_language_label(value))
    {
        return Ok(language.trim().to_string());
    }

    let requested = language_lookup_key(language);
    let requested_primary = primary_language_subtag(&requested);

    model_info
        .languages
        .iter()
        .find(|available| available.trim().eq_ignore_ascii_case(language.trim()))
        .or_else(|| {
            model_info
                .languages
                .iter()
                .find(|available| language_lookup_key(available) == requested)
        })
        .or_else(|| {
            model_info.languages.iter().find(|available| {
                primary_language_subtag(&language_lookup_key(available)) == requested_primary
            })
        })
        .map(|available| available.trim().to_string())
        .ok_or_else(|| unsupported_language_error(language, model_info))
}

fn resolve_voice_for_model(voice: &str, model_info: &AnyModelInfo) -> Result<String> {
    if model_info.voices.is_empty() {
        return Err(unsupported_voice_error(voice, model_info));
    }

    let requested = voice.trim();
    let requested_lower = requested.to_ascii_lowercase();

    model_info
        .voices
        .iter()
        .find(|available| available.trim() == requested)
        .or_else(|| {
            model_info
                .voices
                .iter()
                .find(|available| available.trim().to_ascii_lowercase() == requested_lower)
        })
        .or_else(|| {
            model_info.voices.iter().find(|available| {
                available
                    .trim()
                    .to_ascii_lowercase()
                    .contains(&requested_lower)
            })
        })
        .map(|available| available.trim().to_string())
        .ok_or_else(|| unsupported_voice_error(voice, model_info))
}

fn is_permissive_language_label(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "auto" | "multilingual"
    )
}

fn tts_synthesis_error(model: &dyn TtsModel, request: &SynthesisRequest, error: TtsError) -> Error {
    let model_info = model.model_info();
    match &error {
        TtsError::UnsupportedLanguage(language) => {
            unsupported_language_error(language, &model_info)
        }
        TtsError::UnknownVoice(voice) => unsupported_voice_error(voice, &model_info),
        _ if should_include_available_options(request, &error) => {
            synthesis_error_with_options(&error, &model_info)
        }
        _ => anyhow!("TTS synthesis failed: {}", error),
    }
}

fn should_include_available_options(request: &SynthesisRequest, error: &TtsError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    (request.language.is_some() && message.contains("language"))
        || (request.voice.is_some() && message.contains("voice"))
}

fn synthesis_error_with_options(error: &TtsError, model_info: &AnyModelInfo) -> Error {
    anyhow!(
        "TTS synthesis failed: {}. Available languages: {}. Available voices: {}.",
        error,
        format_available_values(&model_info.languages),
        format_available_values(&model_info.voices)
    )
}

fn unsupported_language_error(language: &str, model_info: &AnyModelInfo) -> Error {
    anyhow!(
        "TTS synthesis failed: unsupported language '{}'. Available languages: {}. Available voices: {}.",
        language,
        format_available_values(&model_info.languages),
        format_available_values(&model_info.voices)
    )
}

fn unsupported_voice_error(voice: &str, model_info: &AnyModelInfo) -> Error {
    anyhow!(
        "TTS synthesis failed: unsupported voice '{}'. Available languages: {}. Available voices: {}.",
        voice,
        format_available_values(&model_info.languages),
        format_available_values(&model_info.voices)
    )
}

fn format_available_values(values: &[String]) -> String {
    let mut values = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();

    if values.is_empty() {
        "none reported by model".to_string()
    } else {
        values.join(", ")
    }
}

fn synthesize_kokoro_chunked(
    model: &dyn TtsModel,
    request: &SynthesisRequest,
) -> Result<AudioSamples> {
    let chunks = split_kokoro_text(&request.text);
    if chunks.len() == 1 {
        return synthesize_kokoro_chunk(model, request, &chunks[0]);
    }

    let mut audio_chunks = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        audio_chunks.push(synthesize_kokoro_chunk(model, request, &chunk)?);
    }

    concatenate_audio_chunks(audio_chunks)
}

fn synthesize_kokoro_chunk(
    model: &dyn TtsModel,
    request: &SynthesisRequest,
    text: &str,
) -> Result<AudioSamples> {
    let mut chunk_request = request.clone();
    chunk_request.text = text.to_string();

    match model.synthesize(&chunk_request) {
        Ok(audio) => Ok(audio),
        Err(error) => {
            let message = error.to_string();
            if is_kokoro_context_limit_error(&message) {
                if let Some((left, right)) = split_text_near_half(text) {
                    let left_audio = synthesize_kokoro_chunk(model, request, &left)?;
                    let right_audio = synthesize_kokoro_chunk(model, request, &right)?;
                    return concatenate_audio_chunks(vec![left_audio, right_audio]);
                }

                return Err(anyhow!(
                    "TTS synthesis failed: Kokoro input is too long for its 512-token context after phonemization. Split the text into shorter phrases or use a TTS model with a longer context."
                ));
            }

            Err(tts_synthesis_error(model, &chunk_request, error))
        }
    }
}

fn split_kokoro_text(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let chunks = if looks_like_markdown(trimmed) {
        split_markdown_text(trimmed)
    } else {
        split_plain_text(trimmed)
    };

    normalize_tts_chunks(chunks)
}

fn split_markdown_text(text: &str) -> Vec<String> {
    let splitter = MarkdownSplitter::new(ChunkConfig::new(KOKORO_TEXT_CHUNK_CHARS));
    let chunks = splitter
        .chunks(text)
        .map(|chunk| chunk.trim().to_string())
        .filter(|chunk| !chunk.is_empty())
        .collect::<Vec<_>>();

    if chunks.is_empty() {
        vec![text.to_string()]
    } else {
        chunks
    }
}

fn split_plain_text(text: &str) -> Vec<String> {
    let splitter = TextSplitter::new(ChunkConfig::new(KOKORO_TEXT_CHUNK_CHARS));
    let chunks = splitter
        .chunks(text)
        .map(|chunk| chunk.trim().to_string())
        .filter(|chunk| !chunk.is_empty())
        .collect::<Vec<_>>();

    if chunks.is_empty() {
        vec![text.to_string()]
    } else {
        chunks
    }
}

fn normalize_tts_chunks(chunks: Vec<String>) -> Vec<String> {
    chunks
        .into_iter()
        .flat_map(|chunk| split_text_sensitively(&chunk, KOKORO_TEXT_CHUNK_CHARS))
        .collect()
}

fn split_text_sensitively(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut remaining = text.trim();

    while char_count(remaining) > max_chars {
        let split_at = best_split_before_limit(remaining, max_chars)
            .unwrap_or_else(|| byte_index_at_char(remaining, max_chars));
        let (left, right) = remaining.split_at(split_at);
        let left = left.trim();

        if !left.is_empty() {
            chunks.push(left.to_string());
        }
        remaining = right.trim();
    }

    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }

    chunks
}

fn looks_like_markdown(text: &str) -> bool {
    if text.contains("```") || text.contains("](") || text.contains("**") || text.contains("__") {
        return true;
    }

    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("# ")
            || trimmed.starts_with("## ")
            || trimmed.starts_with("### ")
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("> ")
            || trimmed.starts_with("| ")
            || is_ordered_markdown_item(trimmed)
    })
}

fn is_ordered_markdown_item(line: &str) -> bool {
    let Some((number, rest)) = line.split_once('.') else {
        return false;
    };

    !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit()) && rest.starts_with(' ')
}

fn split_text_near_half(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim();
    let total_chars = char_count(trimmed);
    if total_chars <= 1 {
        return None;
    }

    let midpoint = total_chars / 2;
    let split_at = best_boundary_split_near(trimmed, midpoint)
        .unwrap_or_else(|| byte_index_at_char(trimmed, midpoint));
    let left = trimmed[..split_at].trim().to_string();
    let right = trimmed[split_at..].trim().to_string();

    if left.is_empty() || right.is_empty() {
        return None;
    }

    Some((left, right))
}

fn best_split_before_limit(text: &str, max_chars: usize) -> Option<usize> {
    let min_chars = (max_chars / 3).max(1);
    let mut preferred = None;
    let mut whitespace = None;
    let mut char_index = 0usize;

    for (byte_index, ch) in text.char_indices() {
        if char_index > max_chars {
            break;
        }

        let next_char_index = char_index + 1;
        let split_at = byte_index + ch.len_utf8();
        if next_char_index >= min_chars && is_preferred_split_char(ch) {
            preferred = Some(split_at);
        } else if next_char_index >= min_chars && ch.is_whitespace() {
            whitespace = Some(split_at);
        }

        char_index = next_char_index;
    }

    preferred.or(whitespace)
}

fn best_boundary_split_near(text: &str, midpoint: usize) -> Option<usize> {
    let mut best = None;
    let mut best_distance = usize::MAX;
    let mut char_index = 0usize;

    for (byte_index, ch) in text.char_indices() {
        let next_char_index = char_index + 1;
        if (is_preferred_split_char(ch) || ch.is_whitespace()) && char_index > 0 {
            let distance = next_char_index.abs_diff(midpoint);
            if distance < best_distance {
                best = Some(byte_index + ch.len_utf8());
                best_distance = distance;
            }
        }
        char_index = next_char_index;
    }

    best
}

fn is_preferred_split_char(ch: char) -> bool {
    matches!(
        ch,
        '.' | '!' | '?' | '\n' | ';' | ':' | ',' | '。' | '！' | '？' | '；' | '：' | '，'
    )
}

fn byte_index_at_char(text: &str, target: usize) -> usize {
    text.char_indices()
        .nth(target)
        .map(|(byte_index, _)| byte_index)
        .unwrap_or(text.len())
}

fn char_count(text: &str) -> usize {
    text.chars().count()
}

fn is_kokoro_context_limit_error(message: &str) -> bool {
    message.contains("index-select invalid index 512 with dim size 512")
}

fn concatenate_audio_chunks(chunks: Vec<AudioSamples>) -> Result<AudioSamples> {
    let Some(first) = chunks.first() else {
        return Err(anyhow!(
            "TTS synthesis failed: model returned no audio chunks"
        ));
    };

    let sample_rate = first.sample_rate;
    let channels = first.channels;
    let pause_samples = (sample_rate as f32 * KOKORO_CHUNK_PAUSE_SECS) as usize * channels as usize;
    let mut samples = Vec::new();

    for chunk in chunks {
        if chunk.sample_rate != sample_rate || chunk.channels != channels {
            return Err(anyhow!(
                "TTS synthesis failed: model returned incompatible audio chunks"
            ));
        }

        if !samples.is_empty() && pause_samples > 0 {
            samples.extend(std::iter::repeat_n(0.0, pause_samples));
        }
        samples.extend(chunk.samples);
    }

    Ok(AudioSamples {
        samples,
        sample_rate,
        channels,
    })
}

fn build_synthesis_request(request: LocalTtsSynthesisRequest) -> Result<SynthesisRequest> {
    let mut synthesis_request = SynthesisRequest::new(request.text);

    if let Some(language) = clean_language(request.language) {
        synthesis_request = synthesis_request.with_language(language);
    }
    if let Some(voice) = clean_defaultable_optional(request.voice) {
        synthesis_request = synthesis_request.with_voice(voice);
    }
    if let Some(instruct) = clean_text_optional(request.instruct) {
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

fn clean_language(value: Option<String>) -> Option<String> {
    clean_defaultable_optional(value)
}

fn clean_defaultable_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && !is_default_option(value))
}

fn clean_text_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn is_default_option(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "auto" | "default" | "none" | "null"
    )
}

fn language_lookup_key(language: &str) -> String {
    let normalized = language.trim().replace('_', "-").to_ascii_lowercase();
    let normalized = normalized
        .replace(['(', ')'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");

    match normalized.as_str() {
        "american-english" | "english-us" | "en-us" => "en-us",
        "british-english" | "english-uk" | "english-gb" | "en-uk" | "en-gb" => "en-gb",
        "english" | "eng" | "en" => "en",
        "arabic" | "ar" => "ar",
        "chinese" | "mandarin" | "mandarin-chinese" | "cmn" | "zh-cn" | "zh-hans" | "zh" => "zh",
        "dutch" | "nl" => "nl",
        "deutsch" | "german" | "de" => "de",
        "french" | "fr" => "fr",
        "hindi" | "hi" => "hi",
        "italian" | "it" => "it",
        "japanese" | "jp" | "ja" => "ja",
        "korean" | "kr" | "ko" => "ko",
        "portuguese" | "pt" | "pt-br" | "pt-pt" => "pt",
        "russian" | "ru" => "ru",
        "spanish" | "es" => "es",
        "multilingual" => "multilingual",
        "auto" => "auto",
        _ => normalized.as_str(),
    }
    .to_string()
}

fn primary_language_subtag(language: &str) -> &str {
    language
        .split_once('-')
        .map_or(language, |(primary, _)| primary)
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
