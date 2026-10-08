//! Model-independent embedding inputs and the contract used by local and remote adapters.

use std::{collections::BTreeMap, fmt, sync::Arc};

use async_trait::async_trait;
use image::DynamicImage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingModality {
    Text,
    Image,
    Audio,
    Video,
}

#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EmbeddingPurpose {
    #[default]
    Query,
    Document,
    Similarity,
    Classification,
    Clustering,
    CodeQuery,
}

/// Interleaved PCM samples. The model adapter owns resampling and feature extraction.
#[derive(Clone, Debug)]
pub struct AudioInput {
    pub samples: Arc<[f32]>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioInput {
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 || self.channels == 0 {
            return 0;
        }
        (self.samples.len() as u64 / u64::from(self.channels)).saturating_mul(1_000)
            / u64::from(self.sample_rate)
    }

    pub fn validate(&self) -> Result<(), EmbeddingError> {
        if self.sample_rate == 0 || self.channels == 0 || self.samples.is_empty() {
            return Err(EmbeddingError::InvalidInput(
                "Audio needs samples, a sample rate, and channels".into(),
            ));
        }
        if !self
            .samples
            .len()
            .is_multiple_of(usize::from(self.channels))
            || self.samples.iter().any(|sample| !sample.is_finite())
        {
            return Err(EmbeddingError::InvalidInput(
                "Audio must contain complete, finite sample frames".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub timestamp_ms: u64,
    pub image: Arc<DynamicImage>,
}

/// Frames remain in timeline order. Supplying audio explicitly includes the audio track.
#[derive(Clone, Debug)]
pub struct VideoInput {
    pub frames: Vec<VideoFrame>,
    pub duration_ms: u64,
    pub audio: Option<AudioInput>,
}

#[derive(Clone, Debug)]
pub enum EmbeddingPart {
    Text(String),
    Image(Arc<DynamicImage>),
    Audio(AudioInput),
    Video(VideoInput),
    /// An encoded media file or HTTP(S) URL, passed intact to a remote adapter.
    EncodedMedia {
        modality: EmbeddingModality,
        source: String,
    },
}

impl EmbeddingPart {
    pub fn modality(&self) -> EmbeddingModality {
        match self {
            Self::Text(_) => EmbeddingModality::Text,
            Self::Image(_) => EmbeddingModality::Image,
            Self::Audio(_) => EmbeddingModality::Audio,
            Self::Video(_) => EmbeddingModality::Video,
            Self::EncodedMedia { modality, .. } => *modality,
        }
    }
}

/// One item becomes one vector. Its ordered parts are encoded jointly.
#[derive(Clone, Debug)]
pub struct EmbeddingInput {
    pub parts: Vec<EmbeddingPart>,
    pub title: Option<String>,
}

impl EmbeddingInput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            parts: vec![EmbeddingPart::Text(text.into())],
            title: None,
        }
    }

    pub fn image(image: DynamicImage) -> Self {
        Self {
            parts: vec![EmbeddingPart::Image(Arc::new(image))],
            title: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingOptions {
    /// Only dimensions declared by the model are accepted. Reduced vectors are normalized again.
    pub dimensions: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct EmbeddingRequest {
    pub items: Vec<EmbeddingInput>,
    pub purpose: EmbeddingPurpose,
    pub options: EmbeddingOptions,
}

impl EmbeddingRequest {
    pub fn texts(texts: impl IntoIterator<Item = String>, purpose: EmbeddingPurpose) -> Self {
        Self {
            items: texts.into_iter().map(EmbeddingInput::text).collect(),
            purpose,
            options: EmbeddingOptions::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingSpace {
    /// Identifies a validated compatible query/document space, including the output recipe.
    pub id: String,
    pub dimensions: usize,
    pub normalized: bool,
    pub metric: EmbeddingMetric,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingMetric {
    #[default]
    Cosine,
    DotProduct,
    Euclidean,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingLimits {
    pub max_tokens: usize,
    pub max_batch_items: Option<usize>,
    pub max_images_per_item: Option<usize>,
    pub max_audio_duration_ms: Option<u64>,
    pub max_video_frames: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingDescriptor {
    pub model_id: String,
    pub adapter: String,
    pub modalities: Vec<EmbeddingModality>,
    /// Exact sets of modalities supported in a joint item. Single modalities use `modalities`.
    pub joint_combinations: Vec<Vec<EmbeddingModality>>,
    pub purposes: Vec<EmbeddingPurpose>,
    pub space: EmbeddingSpace,
    pub supported_dimensions: Vec<usize>,
    pub limits: EmbeddingLimits,
    /// Hash of resolved weights, tokenizer, preprocessing, and output configuration.
    pub pipeline_fingerprint: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingUsage {
    pub input_items: usize,
    pub text_tokens: Option<u64>,
    pub audio_duration_ms: u64,
    pub video_frames: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingProvenance {
    pub model_id: String,
    pub adapter: String,
    pub pipeline_fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingBatch {
    pub embeddings: Vec<Vec<f32>>,
    pub space: EmbeddingSpace,
    pub provenance: EmbeddingProvenance,
    pub usage: EmbeddingUsage,
}

#[derive(Debug)]
pub enum EmbeddingError {
    UnsupportedModality(EmbeddingModality),
    UnsupportedCombination(Vec<EmbeddingModality>),
    UnsupportedPurpose(EmbeddingPurpose),
    UnsupportedDimensions(usize),
    InvalidInput(String),
    LimitExceeded(String),
    InvalidOutput(String),
    Backend(anyhow::Error),
}

impl fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedModality(value) => {
                write!(f, "Model does not support {value:?} inputs")
            }
            Self::UnsupportedCombination(value) => {
                write!(f, "Model does not support joint {value:?} inputs")
            }
            Self::UnsupportedPurpose(value) => {
                write!(f, "Model does not support the {value:?} task")
            }
            Self::UnsupportedDimensions(value) => {
                write!(f, "Model does not support {value} output dimensions")
            }
            Self::InvalidInput(value) | Self::LimitExceeded(value) | Self::InvalidOutput(value) => {
                f.write_str(value)
            }
            Self::Backend(error) => write!(f, "Embedding inference failed: {error:#}"),
        }
    }
}

impl std::error::Error for EmbeddingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

#[async_trait]
pub trait EmbeddingModel: Send + Sync {
    fn descriptor(&self) -> &EmbeddingDescriptor;
    /// Remote adapters can receive original encoded files without decoding media on the caller.
    fn supports_encoded_media(&self) -> bool {
        false
    }
    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingBatch, EmbeddingError>;
}

impl EmbeddingDescriptor {
    pub fn validate(&self, request: &EmbeddingRequest) -> Result<(), EmbeddingError> {
        if !self.purposes.contains(&request.purpose) {
            return Err(EmbeddingError::UnsupportedPurpose(request.purpose));
        }
        if let Some(dimensions) = request.options.dimensions {
            self.validate_dimensions(dimensions)?;
        }
        if self
            .limits
            .max_batch_items
            .is_some_and(|max| request.items.len() > max)
        {
            return Err(EmbeddingError::LimitExceeded(
                "Embedding batch has too many items".into(),
            ));
        }
        for item in &request.items {
            if item.parts.is_empty() {
                return Err(EmbeddingError::InvalidInput(
                    "An embedding item needs at least one content part".into(),
                ));
            }
            let mut modalities = Vec::new();
            let mut images = 0;
            let mut audio_ms = 0u64;
            for part in &item.parts {
                let modality = part.modality();
                if !self.modalities.contains(&modality) {
                    return Err(EmbeddingError::UnsupportedModality(modality));
                }
                modalities.push(modality);
                match part {
                    EmbeddingPart::Text(_) => {}
                    EmbeddingPart::EncodedMedia { modality, source } => {
                        if *modality == EmbeddingModality::Text || source.trim().is_empty() {
                            return Err(EmbeddingError::InvalidInput(
                                "Encoded media needs an image, audio, or video source".into(),
                            ));
                        }
                        if *modality == EmbeddingModality::Image {
                            images += 1;
                        }
                    }
                    EmbeddingPart::Image(image) => {
                        validate_image(image)?;
                        images += 1;
                    }
                    EmbeddingPart::Audio(audio) => {
                        audio.validate()?;
                        audio_ms = audio_ms.saturating_add(audio.duration_ms());
                    }
                    EmbeddingPart::Video(video) => {
                        if video.frames.is_empty() || video.duration_ms == 0 {
                            return Err(EmbeddingError::InvalidInput(
                                "Video needs frames and a duration".into(),
                            ));
                        }
                        if self
                            .limits
                            .max_video_frames
                            .is_some_and(|max| video.frames.len() > max)
                        {
                            return Err(EmbeddingError::LimitExceeded(
                                "Video has too many frames".into(),
                            ));
                        }
                        let mut previous = None;
                        for frame in &video.frames {
                            validate_image(&frame.image)?;
                            if frame.timestamp_ms > video.duration_ms
                                || previous.is_some_and(|time| frame.timestamp_ms <= time)
                            {
                                return Err(EmbeddingError::InvalidInput(
                                    "Video timestamps must be ordered and within its duration"
                                        .into(),
                                ));
                            }
                            previous = Some(frame.timestamp_ms);
                        }
                        if let Some(audio) = &video.audio {
                            if !self.modalities.contains(&EmbeddingModality::Audio) {
                                return Err(EmbeddingError::UnsupportedModality(
                                    EmbeddingModality::Audio,
                                ));
                            }
                            audio.validate()?;
                            audio_ms = audio_ms.saturating_add(audio.duration_ms());
                            modalities.push(EmbeddingModality::Audio);
                        }
                    }
                }
            }
            modalities.sort();
            modalities.dedup();
            if modalities.len() > 1
                && !self.joint_combinations.iter().any(|allowed| {
                    let mut allowed = allowed.clone();
                    allowed.sort();
                    allowed.dedup();
                    allowed == modalities
                })
            {
                return Err(EmbeddingError::UnsupportedCombination(modalities));
            }
            if self
                .limits
                .max_images_per_item
                .is_some_and(|max| images > max)
            {
                return Err(EmbeddingError::LimitExceeded(
                    "Embedding item has too many images".into(),
                ));
            }
            if self
                .limits
                .max_audio_duration_ms
                .is_some_and(|max| audio_ms > max)
            {
                return Err(EmbeddingError::LimitExceeded(
                    "Embedding audio exceeds the duration limit".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn finish(
        &self,
        request: &EmbeddingRequest,
        mut embeddings: Vec<Vec<f32>>,
    ) -> Result<EmbeddingBatch, EmbeddingError> {
        if embeddings.len() != request.items.len() {
            return Err(EmbeddingError::InvalidOutput(format!(
                "Model returned {} vectors for {} inputs",
                embeddings.len(),
                request.items.len()
            )));
        }
        let dimensions = request.options.dimensions.unwrap_or(self.space.dimensions);
        self.validate_dimensions(dimensions)?;
        for vector in &mut embeddings {
            if vector.len() != self.space.dimensions
                || vector.iter().any(|value| !value.is_finite())
            {
                return Err(EmbeddingError::InvalidOutput(
                    "Model returned an invalid embedding shape or non-finite values".into(),
                ));
            }
            if self.space.normalized {
                let norm = squared_norm(vector).sqrt();
                if norm == 0.0 || (norm - 1.0).abs() > 1e-3 {
                    return Err(EmbeddingError::InvalidOutput("Model declared normalized output but returned a vector without unit length".into()));
                }
            }
            if dimensions != vector.len() {
                vector.truncate(dimensions);
                if self.space.normalized {
                    let norm = squared_norm(vector).sqrt();
                    if norm == 0.0 {
                        return Err(EmbeddingError::InvalidOutput(
                            "Reduced embedding has zero length".into(),
                        ));
                    }
                    vector
                        .iter_mut()
                        .for_each(|value| *value = (f64::from(*value) / norm) as f32);
                }
            }
        }
        let mut space = self.space.clone();
        space.dimensions = dimensions;
        let mut usage = EmbeddingUsage {
            input_items: request.items.len(),
            ..Default::default()
        };
        for part in request.items.iter().flat_map(|item| &item.parts) {
            match part {
                EmbeddingPart::Audio(audio) => usage.audio_duration_ms += audio.duration_ms(),
                EmbeddingPart::Video(video) => {
                    usage.video_frames += video.frames.len();
                    if let Some(audio) = &video.audio {
                        usage.audio_duration_ms += audio.duration_ms();
                    }
                }
                _ => {}
            }
        }
        Ok(EmbeddingBatch {
            embeddings,
            space,
            provenance: EmbeddingProvenance {
                model_id: self.model_id.clone(),
                adapter: self.adapter.clone(),
                pipeline_fingerprint: self.pipeline_fingerprint.clone(),
            },
            usage,
        })
    }

    fn validate_dimensions(&self, dimensions: usize) -> Result<(), EmbeddingError> {
        if dimensions == 0
            || dimensions > self.space.dimensions
            || (dimensions != self.space.dimensions
                && !self.supported_dimensions.contains(&dimensions))
        {
            return Err(EmbeddingError::UnsupportedDimensions(dimensions));
        }
        Ok(())
    }
}

fn squared_norm(vector: &[f32]) -> f64 {
    vector.iter().map(|&value| f64::from(value).powi(2)).sum()
}

fn validate_image(image: &DynamicImage) -> Result<(), EmbeddingError> {
    if image.width() == 0 || image.height() == 0 {
        return Err(EmbeddingError::InvalidInput(
            "Image dimensions must be nonzero".into(),
        ));
    }
    Ok(())
}

/// A Bit dependency assigned to a model role and its graph-relative destination.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingArtifact {
    pub bit: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<EmbeddingArtifactSource>,
}

/// A downloadable artifact whose bytes are pinned by a BLAKE3 digest.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingArtifactSource {
    pub url: String,
    pub hash: String,
    pub size: u64,
}

/// Versioned model recipe stored in a Bit's `parameters.embedding` field.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingSpec {
    pub schema_version: u32,
    pub adapter: String,
    #[serde(default = "default_adapter_version")]
    pub adapter_version: u32,
    pub space_id: String,
    pub dimensions: usize,
    #[serde(default)]
    pub supported_dimensions: Vec<usize>,
    pub max_tokens: usize,
    pub artifacts: BTreeMap<String, EmbeddingArtifact>,
    #[serde(default)]
    pub pooling: EmbeddingPooling,
    #[serde(default)]
    pub task_profiles: BTreeMap<EmbeddingPurpose, EmbeddingTaskProfile>,
}

fn default_adapter_version() -> u32 {
    1
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingPooling {
    #[default]
    Mean,
    Cls,
    LastToken,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingTaskProfile {
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
}

impl EmbeddingSpec {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == 1,
            "Unsupported embedding schema version {}",
            self.schema_version
        );
        anyhow::ensure!(
            self.adapter_version == 1,
            "Unsupported embedding adapter version {}",
            self.adapter_version
        );
        anyhow::ensure!(
            !self.space_id.trim().is_empty(),
            "Embedding space ID is required"
        );
        anyhow::ensure!(
            self.dimensions > 0 && self.max_tokens > 0,
            "Embedding dimensions and context must be positive"
        );
        anyhow::ensure!(
            self.supported_dimensions
                .iter()
                .all(|&size| size > 0 && size <= self.dimensions),
            "Invalid reduced embedding dimensions"
        );
        anyhow::ensure!(
            !self.adapter.trim().is_empty(),
            "Embedding adapter is required"
        );
        let mut paths = std::collections::HashSet::new();
        let mut inline_ids = std::collections::HashSet::new();
        for artifact in self.artifacts.values() {
            let path = std::path::Path::new(&artifact.path);
            anyhow::ensure!(
                !artifact.bit.is_empty()
                    && !artifact.path.is_empty()
                    && !artifact.path.contains(['\\', ':'])
                    && path
                        .components()
                        .all(|component| matches!(component, std::path::Component::Normal(_))),
                "Embedding artifact paths must be relative without traversal"
            );
            anyhow::ensure!(
                paths.insert(path.components().collect::<std::path::PathBuf>()),
                "Embedding artifacts cannot share a destination path"
            );
            if let Some(source) = &artifact.source {
                anyhow::ensure!(
                    inline_ids.insert(&artifact.bit),
                    "Inline embedding artifacts must have distinct Bit IDs"
                );
                anyhow::ensure!(
                    source.size > 0
                        && source.hash.len() == 64
                        && source
                            .hash
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                        && source.hash != artifact.bit,
                    "Inline embedding artifacts require a positive size and a lowercase BLAKE3 digest distinct from their Bit ID"
                );
                let url = reqwest::Url::parse(&source.url)?;
                anyhow::ensure!(
                    matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.fragment().is_none(),
                    "Inline embedding artifact URLs must use HTTP or HTTPS without credentials or fragments"
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> EmbeddingDescriptor {
        EmbeddingDescriptor {
            model_id: "test".into(),
            adapter: "test".into(),
            modalities: vec![EmbeddingModality::Text, EmbeddingModality::Image],
            joint_combinations: vec![],
            purposes: vec![EmbeddingPurpose::Query, EmbeddingPurpose::Document],
            space: EmbeddingSpace {
                id: "space".into(),
                dimensions: 3,
                normalized: true,
                metric: EmbeddingMetric::Cosine,
            },
            supported_dimensions: vec![2],
            limits: EmbeddingLimits::default(),
            pipeline_fingerprint: "revision".into(),
        }
    }

    #[test]
    fn independent_modalities_do_not_advertise_joint_encoding() {
        let request = EmbeddingRequest {
            items: vec![EmbeddingInput {
                parts: vec![
                    EmbeddingPart::Text("caption".into()),
                    EmbeddingPart::Image(Arc::new(DynamicImage::new_rgb8(2, 2))),
                ],
                title: None,
            }],
            purpose: EmbeddingPurpose::Document,
            options: EmbeddingOptions::default(),
        };
        assert!(matches!(
            descriptor().validate(&request),
            Err(EmbeddingError::UnsupportedCombination(_))
        ));
        let mut model = descriptor();
        model
            .joint_combinations
            .push(vec![EmbeddingModality::Image, EmbeddingModality::Text]);
        model.validate(&request).unwrap();
    }

    #[test]
    fn dimensions_are_declared_and_reduced_vectors_are_normalized() {
        let mut request = EmbeddingRequest::texts(["hello".into()], EmbeddingPurpose::Query);
        request.options.dimensions = Some(2);
        let batch = descriptor()
            .finish(&request, vec![vec![0.3, 0.4, 0.8660254]])
            .unwrap();
        assert_eq!(batch.space.dimensions, 2);
        assert!((batch.embeddings[0][0] - 0.6).abs() < 1e-6);
        assert!((batch.embeddings[0][1] - 0.8).abs() < 1e-6);
        for dimensions in [0, 1, 4] {
            request.options.dimensions = Some(dimensions);
            assert!(descriptor().validate(&request).is_err());
        }
    }

    #[test]
    fn invalid_output_is_rejected_before_it_reaches_an_index() {
        let request = EmbeddingRequest::texts(["hello".into()], EmbeddingPurpose::Query);
        for vectors in [
            vec![],
            vec![vec![1.0, 2.0]],
            vec![vec![f32::NAN, 0.0, 0.0]],
            vec![vec![0.0; 3]],
            vec![vec![f32::MAX; 3]],
        ] {
            assert!(descriptor().finish(&request, vectors).is_err());
        }
        let mut reduced = request;
        reduced.options.dimensions = Some(2);
        assert!(
            descriptor()
                .finish(&reduced, vec![vec![0.0, 0.0, 1.0]])
                .is_err()
        );
    }

    #[test]
    fn audio_retains_sample_layout_and_rejects_incomplete_frames() {
        let mut audio = AudioInput {
            samples: vec![0.0; 32_000].into(),
            sample_rate: 16_000,
            channels: 2,
        };
        audio.validate().unwrap();
        assert_eq!(audio.duration_ms(), 1_000);
        audio.samples = vec![0.0; 3].into();
        assert!(audio.validate().is_err());
        audio.samples = vec![f32::INFINITY; 2].into();
        assert!(audio.validate().is_err());
    }

    #[test]
    fn recipe_rejects_unknown_versions_and_unsafe_artifact_layouts() {
        let mut spec = EmbeddingSpec {
            schema_version: 1,
            adapter: "embedding_gemma2".into(),
            adapter_version: 1,
            space_id: "gemma2".into(),
            dimensions: 768,
            supported_dimensions: vec![128, 256, 512],
            max_tokens: 8192,
            artifacts: BTreeMap::new(),
            pooling: EmbeddingPooling::Mean,
            task_profiles: BTreeMap::new(),
        };
        spec.validate().unwrap();
        spec.schema_version = 2;
        assert!(spec.validate().is_err());
        spec.schema_version = 1;
        for path in [
            "../weights",
            "/weights",
            "C:\\weights",
            "onnx/../../weights",
        ] {
            spec.artifacts.insert(
                "backbone".into(),
                EmbeddingArtifact {
                    bit: "id".into(),
                    path: path.into(),
                    source: None,
                },
            );
            assert!(spec.validate().is_err(), "{path}");
        }
        spec.artifacts.insert(
            "backbone".into(),
            EmbeddingArtifact {
                bit: "backbone".into(),
                path: "onnx/model.onnx".into(),
                source: None,
            },
        );
        spec.validate().unwrap();
        for alias in ["onnx//model.onnx", "onnx/./model.onnx", "onnx/model.onnx/"] {
            let mut aliased = spec.clone();
            aliased.artifacts.insert(
                "weights".into(),
                EmbeddingArtifact {
                    bit: "weights".into(),
                    path: alias.into(),
                    source: None,
                },
            );
            assert!(aliased.validate().is_err(), "{alias}");
        }
    }

    #[test]
    fn recipe_sources_require_verifiable_content_and_preserve_legacy_json() {
        let legacy = serde_json::json!({"bit": "weights", "path": "onnx/model.onnx"});
        let artifact: EmbeddingArtifact = serde_json::from_value(legacy.clone()).unwrap();
        assert!(artifact.source.is_none());
        assert_eq!(serde_json::to_value(artifact).unwrap(), legacy);
        let mut recipe = serde_json::json!({
            "schema_version": 1, "adapter": "embedding_gemma2", "space_id": "gemma2",
            "dimensions": 768, "max_tokens": 8192,
            "artifacts": {"backbone": {"bit": "weights", "path": "onnx/model.onnx", "source": {
                "url": "https://example.com/model.onnx", "hash": "a".repeat(64), "size": 100
            }}}
        });
        let validate = |value: &serde_json::Value| {
            serde_json::from_value::<EmbeddingSpec>(value.clone())
                .unwrap()
                .validate()
        };
        validate(&recipe).unwrap();
        for (field, value) in [
            ("hash", serde_json::json!("short")),
            ("hash", serde_json::json!("A".repeat(64))),
            ("size", serde_json::json!(0)),
            ("url", serde_json::json!("file:///tmp/model.onnx")),
            (
                "url",
                serde_json::json!("https://user:password@example.com/model.onnx"),
            ),
            (
                "url",
                serde_json::json!("https://example.com/model.onnx#fragment"),
            ),
        ] {
            let mut invalid = recipe.clone();
            invalid["artifacts"]["backbone"]["source"][field] = value;
            assert!(validate(&invalid).is_err(), "{field}");
        }
        recipe["artifacts"]["backbone"]["bit"] = serde_json::json!("a".repeat(64));
        assert!(validate(&recipe).is_err());
        recipe["artifacts"]["backbone"]["bit"] = serde_json::json!("weights");
        recipe["artifacts"]["duplicate"] = recipe["artifacts"]["backbone"].clone();
        recipe["artifacts"]["duplicate"]["path"] = serde_json::json!("other.onnx");
        assert!(validate(&recipe).is_err());
    }
}
