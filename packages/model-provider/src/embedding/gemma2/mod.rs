//! EmbeddingGemma 2's text backbone and media encoders share one embedding space.
//!
//! Processor recipe: Hugging Face Transformers revision
//! 14e738b5d0cc69aa27a95dde272aea41fde44f2f, Gemma4 and EmbeddingGemma2 processors.

mod audio;
mod vision;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail, ensure};
use ort::{session::Session, value::Tensor};
use tokenizers::Tokenizer;

use crate::{
    embedding::interface::{
        AudioInput, EmbeddingInput, EmbeddingPart, EmbeddingPurpose, VideoInput,
    },
    ml::ort_runtime::configured_session_builder,
};

const HIDDEN_SIZE: usize = 512;
const DIMENSIONS: usize = 768;
const IMAGE: &str = "<|image|>";
const VIDEO: &str = "<|video|>";
const AUDIO: &str = "<|audio|>";

#[derive(Clone, Debug)]
pub struct Gemma2Options {
    pub model_file: PathBuf,
    pub vision_file: Option<PathBuf>,
    pub audio_file: Option<PathBuf>,
    pub tokenizer_file: PathBuf,
    pub processor_file: PathBuf,
    pub dimensions: usize,
    pub max_tokens: usize,
    pub image_soft_tokens: usize,
    pub video_soft_tokens: usize,
    pub max_video_frames: usize,
}

impl Default for Gemma2Options {
    fn default() -> Self {
        Self {
            model_file: "onnx/model_q4.onnx".into(),
            vision_file: Some("onnx/vision_encoder_q4.onnx".into()),
            audio_file: Some("onnx/audio_encoder_q4.onnx".into()),
            tokenizer_file: "tokenizer.json".into(),
            processor_file: "processor_config.json".into(),
            dimensions: DIMENSIONS,
            max_tokens: 8192,
            image_soft_tokens: 280,
            video_soft_tokens: 140,
            max_video_frames: 32,
        }
    }
}

pub struct Gemma2Embedding {
    backbone: Session,
    vision: Option<Session>,
    audio: Option<Session>,
    tokenizer: Arc<Tokenizer>,
    options: Gemma2Options,
}

#[derive(Clone)]
pub struct Gemma2Preprocessor {
    tokenizer: Arc<Tokenizer>,
    options: Gemma2Options,
    has_vision: bool,
    has_audio: bool,
}

pub struct Gemma2PreparedInput {
    tokenizer: Arc<Tokenizer>,
    ids: Vec<i64>,
    media: Vec<PreparedMedia>,
}

enum PreparedMedia {
    Image(vision::VisionFeatures),
    Video(vision::VisionFeatures),
    Audio(audio::AudioFeatures),
}

#[derive(Default)]
struct EncodedFeatures {
    images: Vec<f32>,
    videos: Vec<f32>,
    audio: Vec<f32>,
}

impl Gemma2Embedding {
    pub fn load(directory: &Path, options: Gemma2Options) -> Result<Self> {
        ensure!(
            [128, 256, 512, DIMENSIONS].contains(&options.dimensions),
            "EmbeddingGemma 2 supports 128, 256, 512 or 768 output dimensions"
        );
        ensure!(
            options.max_tokens > 0 && options.max_tokens <= 8192,
            "EmbeddingGemma 2's supported context is at most 8192 tokens"
        );
        ensure!(
            options.max_video_frames > 0 && options.max_video_frames <= 8192 / 70,
            "Invalid video frame budget"
        );
        for budget in [options.image_soft_tokens, options.video_soft_tokens] {
            ensure!(
                [70, 140, 280, 560, 1120].contains(&budget),
                "Unsupported vision token budget {budget}"
            );
        }
        validate_processor(&directory.join(&options.processor_file))?;
        let mut tokenizer = Tokenizer::from_file(directory.join(&options.tokenizer_file))
            .map_err(|error| anyhow::anyhow!("Cannot load EmbeddingGemma 2 tokenizer: {error}"))?;
        // Truncating expanded media placeholders would disconnect tokens from feature rows.
        tokenizer
            .with_truncation(None)
            .map_err(|error| anyhow::anyhow!("Cannot configure tokenizer: {error}"))?;
        tokenizer.with_padding(None);
        for (token, id) in [
            (IMAGE, 258880),
            (AUDIO, 258881),
            (VIDEO, 258884),
            ("<|image>", 255999),
            ("<image|>", 258882),
            ("<|audio>", 256000),
            ("<audio|>", 258883),
        ] {
            ensure!(
                tokenizer.token_to_id(token) == Some(id),
                "Tokenizer does not match EmbeddingGemma 2: {token}"
            );
        }
        let backbone = load_session(&directory.join(&options.model_file))?;
        validate_inputs(
            &backbone,
            &[
                "input_ids",
                "attention_mask",
                "image_features",
                "video_features",
                "audio_features",
            ],
        )?;
        let vision = options
            .vision_file
            .as_ref()
            .map(|file| load_session(&directory.join(file)))
            .transpose()?;
        if let Some(session) = &vision {
            validate_inputs(session, &["pixel_values", "pixel_position_ids"])?;
        }
        let audio = options
            .audio_file
            .as_ref()
            .map(|file| load_session(&directory.join(file)))
            .transpose()?;
        if let Some(session) = &audio {
            validate_inputs(session, &["input_features", "input_features_mask"])?;
        }
        Ok(Self {
            backbone,
            vision,
            audio,
            tokenizer: Arc::new(tokenizer),
            options,
        })
    }

    pub fn embed(
        &mut self,
        inputs: &[EmbeddingInput],
        purpose: EmbeddingPurpose,
    ) -> Result<Vec<Vec<f32>>> {
        // Execute each item independently to bound media-encoder memory and avoid padded
        // audio changing a shorter item's valid-feature mask.
        let preprocessor = self.preprocessor();
        inputs
            .iter()
            .enumerate()
            .map(|(index, input)| {
                preprocessor
                    .prepare(input, purpose)
                    .and_then(|prepared| self.embed_prepared(prepared))
                    .with_context(|| format!("EmbeddingGemma 2 input {index}"))
            })
            .collect()
    }

    pub fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }

    pub fn preprocessor(&self) -> Gemma2Preprocessor {
        Gemma2Preprocessor {
            tokenizer: Arc::clone(&self.tokenizer),
            options: self.options.clone(),
            has_vision: self.vision.is_some(),
            has_audio: self.audio.is_some(),
        }
    }

    #[cfg(test)]
    fn input_ids(&self, input: &EmbeddingInput, purpose: EmbeddingPurpose) -> Result<Vec<i64>> {
        self.preprocessor().input_ids(input, purpose)
    }

    pub fn embed_prepared(&mut self, input: Gemma2PreparedInput) -> Result<Vec<f32>> {
        ensure!(
            Arc::ptr_eq(&input.tokenizer, &self.tokenizer),
            "Prepared input belongs to a different EmbeddingGemma 2 model"
        );
        let Gemma2PreparedInput { ids, media, .. } = input;
        let mut prepared = EncodedFeatures::default();
        for part in media {
            match part {
                PreparedMedia::Image(features) => {
                    prepared.images.extend(self.encode_image(features)?);
                }
                PreparedMedia::Video(features) => {
                    prepared.videos.extend(self.encode_image(features)?);
                }
                PreparedMedia::Audio(features) => self.append_audio(&mut prepared, features)?,
            }
        }
        for (token, features) in [
            (258880, &prepared.images),
            (258884, &prepared.videos),
            (258881, &prepared.audio),
        ] {
            ensure!(
                ids.iter().filter(|id| **id == token).count() == features.len() / HIDDEN_SIZE,
                "Media token count does not match encoded features"
            );
        }
        let sequence = ids.len();
        let outputs = self.backbone.run(ort::inputs! {
            "input_ids" => Tensor::from_array(([1, sequence], ids))?,
            "attention_mask" => Tensor::from_array(([1, sequence], vec![1_i64; sequence]))?,
            "image_features" => feature_tensor(prepared.images)?,
            "video_features" => feature_tensor(prepared.videos)?,
            "audio_features" => feature_tensor(prepared.audio)?,
        })?;
        let output = outputs
            .get("sentence_embedding")
            .context("Backbone has no sentence_embedding output")?;
        let (shape, values) = output.try_extract_tensor::<f32>()?;
        ensure!(
            shape.as_ref() == [1, DIMENSIONS as i64],
            "Unexpected sentence_embedding shape {shape:?}"
        );
        let mut vector = values[..self.options.dimensions].to_vec();
        ensure!(
            vector.iter().all(|value| value.is_finite()),
            "Embedding contains non-finite values"
        );
        let norm = vector
            .iter()
            .map(|value| (*value as f64).powi(2))
            .sum::<f64>()
            .sqrt();
        ensure!(norm > 0.0, "Embedding has zero norm");
        for value in &mut vector {
            *value = (*value as f64 / norm) as f32;
        }
        Ok(vector)
    }

    fn encode_image(&mut self, features: vision::VisionFeatures) -> Result<Vec<f32>> {
        let encoder = self
            .vision
            .as_mut()
            .context("This EmbeddingGemma 2 pack has no vision encoder")?;
        let outputs = encoder.run(ort::inputs! {
            "pixel_values" => Tensor::from_array(([1, features.patches, vision::PATCH_VALUES], features.pixels))?,
            "pixel_position_ids" => Tensor::from_array(([1, features.patches, 2], features.positions))?,
        })?;
        let (shape, values) = outputs
            .get("image_features")
            .context("Vision encoder has no image_features output")?
            .try_extract_tensor::<f32>()?;
        ensure!(
            shape.as_ref() == [features.tokens as i64, HIDDEN_SIZE as i64],
            "Unexpected image_features shape {shape:?}"
        );
        Ok(values.to_vec())
    }

    fn append_audio(
        &mut self,
        prepared: &mut EncodedFeatures,
        features: audio::AudioFeatures,
    ) -> Result<()> {
        let encoder = self
            .audio
            .as_mut()
            .context("This EmbeddingGemma 2 pack has no audio encoder")?;
        let frames = features.mask.len();
        let outputs = encoder.run(ort::inputs! {
            "input_features" => Tensor::from_array(([1, frames, audio::MEL_BINS], features.values))?,
            "input_features_mask" => Tensor::from_array(([1, frames], features.mask))?,
        })?;
        let (shape, values) = outputs
            .get("audio_features")
            .context("Audio encoder has no audio_features output")?
            .try_extract_tensor::<f32>()?;
        ensure!(
            shape.as_ref() == [features.tokens as i64, HIDDEN_SIZE as i64],
            "Unexpected audio_features shape {shape:?}"
        );
        prepared.audio.extend_from_slice(values);
        Ok(())
    }
}

impl Gemma2Preprocessor {
    /// Prepares one item without accessing model sessions. Callers should bound queued items.
    pub fn prepare(
        &self,
        input: &EmbeddingInput,
        purpose: EmbeddingPurpose,
    ) -> Result<Gemma2PreparedInput> {
        // Count expanded placeholders and tokenize before image resizing, FFTs or encoder inference.
        let ids = self.input_ids(input, purpose)?;
        let mut media = Vec::new();
        for part in &input.parts {
            match part {
                EmbeddingPart::Text(_) => {}
                EmbeddingPart::EncodedMedia { .. } => {
                    anyhow::bail!("Local embedding models require decoded media inputs");
                }
                EmbeddingPart::Image(image) => media.push(PreparedMedia::Image(
                    vision::preprocess(image, self.options.image_soft_tokens)?,
                )),
                EmbeddingPart::Audio(input) => {
                    media.push(PreparedMedia::Audio(audio::preprocess(input)?));
                }
                EmbeddingPart::Video(video) => {
                    for index in sample_video(video, self.options.max_video_frames)? {
                        media.push(PreparedMedia::Video(vision::preprocess(
                            &video.frames[index].image,
                            self.options.video_soft_tokens,
                        )?));
                    }
                    if let Some(input) = &video.audio {
                        media.push(PreparedMedia::Audio(audio::preprocess(input)?));
                    }
                }
            }
        }
        Ok(Gemma2PreparedInput {
            tokenizer: Arc::clone(&self.tokenizer),
            ids,
            media,
        })
    }

    fn input_ids(&self, input: &EmbeddingInput, purpose: EmbeddingPurpose) -> Result<Vec<i64>> {
        ensure!(!input.parts.is_empty(), "Embedding input has no parts");
        let mut text = input_prefix(input, purpose)?;
        let mut media_tokens = 0;
        let append_audio = |text: &mut String, input: &AudioInput| -> Result<usize> {
            ensure!(
                self.has_audio,
                "This EmbeddingGemma 2 pack has no audio encoder"
            );
            ensure!(
                input.duration_ms() <= self.options.max_tokens as u64 * 40,
                "Audio exceeds the model context"
            );
            let count = audio::token_count(input)?;
            text.push_str("<|audio>");
            for _ in 0..count {
                text.push_str(AUDIO);
            }
            text.push_str("<audio|>");
            Ok(count)
        };
        for part in &input.parts {
            match part {
                EmbeddingPart::Text(value) => {
                    validate_text(value)?;
                    text.push_str(value);
                }
                EmbeddingPart::EncodedMedia { .. } => {
                    anyhow::bail!("Local embedding models require decoded media inputs");
                }
                EmbeddingPart::Image(image) => {
                    ensure!(
                        self.has_vision,
                        "This EmbeddingGemma 2 pack has no vision encoder"
                    );
                    let count = vision::token_count(image, self.options.image_soft_tokens)?;
                    append_visual(&mut text, IMAGE, count);
                    media_tokens += count;
                }
                EmbeddingPart::Audio(audio) => {
                    media_tokens += append_audio(&mut text, audio)?;
                }
                EmbeddingPart::Video(video) => {
                    ensure!(
                        self.has_vision,
                        "This EmbeddingGemma 2 pack has no vision encoder"
                    );
                    for index in sample_video(video, self.options.max_video_frames)? {
                        let count = vision::token_count(
                            &video.frames[index].image,
                            self.options.video_soft_tokens,
                        )?;
                        append_visual(&mut text, VIDEO, count);
                        media_tokens += count;
                    }
                    if let Some(audio) = &video.audio {
                        media_tokens += append_audio(&mut text, audio)?;
                    }
                }
            }
            ensure!(
                media_tokens <= self.options.max_tokens,
                "Media exceeds the {}-token context",
                self.options.max_tokens
            );
        }
        let encoding = self
            .tokenizer
            .encode(text.as_str(), true)
            .map_err(|error| anyhow::anyhow!("Tokenization failed: {error}"))?;
        ensure!(
            encoding.len() <= self.options.max_tokens,
            "Input uses {} tokens, exceeding the {}-token context",
            encoding.len(),
            self.options.max_tokens
        );
        Ok(encoding.get_ids().iter().map(|id| i64::from(*id)).collect())
    }
}

fn validate_text(text: &str) -> Result<()> {
    ensure!(
        ![
            IMAGE, VIDEO, AUDIO, "<|image>", "<image|>", "<|audio>", "<audio|>"
        ]
        .iter()
        .any(|token| text.contains(token)),
        "Text contains reserved media tokens; provide structured media parts instead"
    );
    Ok(())
}

fn feature_tensor(values: Vec<f32>) -> ort::Result<Tensor<f32>> {
    if values.is_empty() {
        // ORT accepts an empty modality. Its Rust raw-data constructor rejects zero
        // dimensions, so use the allocator constructor for this zero-element tensor.
        Tensor::new(&ort::memory::Allocator::default(), [0, HIDDEN_SIZE])
    } else {
        Tensor::from_array(([values.len() / HIDDEN_SIZE, HIDDEN_SIZE], values))
    }
}

fn validate_processor(path: &Path) -> Result<()> {
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(path)
            .with_context(|| format!("Cannot read processor configuration {}", path.display()))?,
    )?;
    ensure!(
        config["processor_class"] == "EmbeddingGemma2Processor",
        "Unsupported EmbeddingGemma processor class"
    );
    for (section, class_key, class) in [
        (
            "image_processor",
            "image_processor_type",
            "Gemma4ImageProcessor",
        ),
        (
            "video_processor",
            "video_processor_type",
            "EmbeddingGemma2VideoProcessor",
        ),
    ] {
        let vision = &config[section];
        ensure!(vision[class_key] == class, "Unsupported {section} class");
        for (key, expected) in [
            ("patch_size", 16),
            ("pooling_kernel_size", 3),
            ("resample", 3),
        ] {
            ensure!(
                vision[key].as_u64() == Some(expected),
                "Unsupported {section}.{key}"
            );
        }
        for (key, expected) in [
            ("do_convert_rgb", true),
            ("do_resize", true),
            ("do_rescale", true),
            ("do_normalize", false),
        ] {
            ensure!(
                vision[key].as_bool() == Some(expected),
                "Unsupported {section}.{key}"
            );
        }
        ensure!(
            vision["rescale_factor"]
                .as_f64()
                .is_some_and(|scale| (scale - 1.0 / 255.0).abs() < 1e-12),
            "Unsupported {section}.rescale_factor"
        );
    }
    let audio = &config["feature_extractor"];
    ensure!(
        audio["feature_extractor_type"] == "Gemma4AudioFeatureExtractor",
        "Unsupported audio feature extractor"
    );
    for (key, expected) in [
        ("sampling_rate", 16000),
        ("feature_size", 128),
        ("frame_length", 320),
        ("hop_length", 160),
        ("fft_length", 512),
    ] {
        ensure!(
            audio[key].as_u64() == Some(expected),
            "Unsupported audio preprocessing {key}"
        );
    }
    for (key, expected) in [
        ("dither", 0.0),
        ("preemphasis", 0.0),
        ("min_frequency", 0.0),
        ("max_frequency", 8000.0),
        ("input_scale_factor", 1.0),
        ("mel_floor", 0.001),
        ("padding_value", 0.0),
    ] {
        ensure!(
            audio[key]
                .as_f64()
                .is_some_and(|value| (value - expected).abs() < 1e-12),
            "Unsupported audio preprocessing {key}"
        );
    }
    ensure!(
        audio["per_bin_mean"].is_null() && audio["per_bin_stddev"].is_null(),
        "Audio per-bin normalization is not supported by this processor recipe"
    );
    ensure!(
        audio["fft_overdrive"] == false && audio["padding_side"] == "right",
        "Unsupported audio FFT or padding recipe"
    );
    ensure!(
        config["video_processor"]["add_timestamps"] == false
            && config["video_processor"]["fps"] == 1
            && config["video_processor"]["overflow_strategy"] == "uniform",
        "Unsupported video sampling recipe"
    );
    Ok(())
}

fn append_visual(text: &mut String, token: &str, count: usize) {
    text.push_str("<|image>");
    for _ in 0..count {
        text.push_str(token);
    }
    text.push_str("<image|>");
}

fn input_prefix(input: &EmbeddingInput, purpose: EmbeddingPurpose) -> Result<String> {
    let has_text = input
        .parts
        .iter()
        .any(|part| matches!(part, EmbeddingPart::Text(_)));
    if let Some(title) = &input.title {
        ensure!(
            has_text && purpose == EmbeddingPurpose::Document,
            "Document titles require a text part and the Document purpose"
        );
        validate_text(title)?;
    }
    Ok(if has_text {
        purpose_prefix(purpose, input.title.as_deref())
    } else {
        String::new()
    })
}

fn purpose_prefix(purpose: EmbeddingPurpose, title: Option<&str>) -> String {
    match purpose {
        EmbeddingPurpose::Query => "task: search result | query: ".into(),
        EmbeddingPurpose::Document => format!(
            "title: {} | text: ",
            title.filter(|title| !title.is_empty()).unwrap_or("none")
        ),
        EmbeddingPurpose::Similarity => "task: sentence similarity | query: ".into(),
        EmbeddingPurpose::Classification => "task: classification | query: ".into(),
        EmbeddingPurpose::Clustering => "task: clustering | query: ".into(),
        EmbeddingPurpose::CodeQuery => "task: code retrieval | query: ".into(),
    }
}

fn load_session(path: &Path) -> Result<Session> {
    // Loading by path lets ORT resolve the export's sibling .onnx_data weights.
    configured_session_builder()?
        .commit_from_file(path)
        .with_context(|| {
            format!(
                "Cannot load ONNX graph {} and its external weights",
                path.display()
            )
        })
}

fn validate_inputs(session: &Session, names: &[&str]) -> Result<()> {
    let actual: Vec<&str> = session.inputs().iter().map(|input| input.name()).collect();
    ensure!(
        actual.len() == names.len() && names.iter().all(|name| actual.contains(name)),
        "Unexpected ONNX graph inputs {actual:?}"
    );
    Ok(())
}

fn sample_video(video: &VideoInput, max_frames: usize) -> Result<Vec<usize>> {
    ensure!(!video.frames.is_empty(), "Video contains no decoded frames");
    ensure!(video.duration_ms > 0, "Video requires a positive duration");
    ensure!(
        video
            .frames
            .windows(2)
            .all(|frames| frames[0].timestamp_ms < frames[1].timestamp_ms),
        "Video frame timestamps must increase"
    );
    ensure!(
        video.frames.last().unwrap().timestamp_ms <= video.duration_ms,
        "Video frame timestamp exceeds its duration"
    );
    let samples = (video.duration_ms / 1000).max(1) as usize;
    // Match reference sampling at 1 fps, then uniform overflow reduction. Select
    // the decoded frame at or immediately before each sample's timeline position.
    let requested: Vec<usize> = if samples > max_frames {
        if max_frames == 1 {
            vec![0]
        } else {
            (0..max_frames)
                .map(|i| i * (samples - 1) / (max_frames - 1))
                .collect()
        }
    } else {
        (0..samples).collect()
    };
    let mut selected = Vec::with_capacity(requested.len());
    for second in requested {
        let timestamp = second as u64 * 1000;
        let index = video
            .frames
            .partition_point(|frame| frame.timestamp_ms <= timestamp)
            .saturating_sub(1);
        selected.push(index);
    }
    if selected.is_empty() {
        bail!("Video sampling selected no frames");
    }
    Ok(selected)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod preparation_tests {
    use super::*;
    use crate::embedding::interface::VideoFrame;
    use image::{DynamicImage, Rgb, RgbImage};
    use tokenizers::{AddedToken, models::bpe::BPE};

    fn preprocessor() -> Gemma2Preprocessor {
        let mut tokenizer = Tokenizer::new(BPE::default());
        tokenizer.add_special_tokens(
            &[
                IMAGE, VIDEO, AUDIO, "<|image>", "<image|>", "<|audio>", "<audio|>",
            ]
            .map(|token| AddedToken::from(token, true)),
        );
        Gemma2Preprocessor {
            tokenizer: Arc::new(tokenizer),
            options: Gemma2Options {
                image_soft_tokens: 70,
                video_soft_tokens: 70,
                ..Gemma2Options::default()
            },
            has_vision: true,
            has_audio: true,
        }
    }

    fn audio_input(samples: usize) -> AudioInput {
        AudioInput {
            samples: (0..samples)
                .map(|index| (index as f32 * 0.2).sin() * 0.1)
                .collect::<Vec<_>>()
                .into(),
            sample_rate: 16_000,
            channels: 1,
        }
    }

    fn assert_vision(actual: &vision::VisionFeatures, expected: &vision::VisionFeatures) {
        assert_eq!(actual.pixels, expected.pixels);
        assert_eq!(actual.positions, expected.positions);
        assert_eq!(actual.patches, expected.patches);
        assert_eq!(actual.tokens, expected.tokens);
    }

    fn assert_audio(actual: &audio::AudioFeatures, expected: &audio::AudioFeatures) {
        assert_eq!(actual.values, expected.values);
        assert_eq!(actual.mask, expected.mask);
        assert_eq!(actual.tokens, expected.tokens);
    }

    #[test]
    fn concurrent_preparation_preserves_joint_media_order_and_audio_masks() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Gemma2Preprocessor>();
        assert_send_sync::<Gemma2PreparedInput>();

        let preprocessor = preprocessor();
        let first_image = Arc::new(DynamicImage::ImageRgb8(RgbImage::from_pixel(
            48,
            48,
            Rgb([25, 80, 140]),
        )));
        let second_image = Arc::new(DynamicImage::ImageRgb8(RgbImage::from_pixel(
            48,
            48,
            Rgb([210, 160, 20]),
        )));
        let short_audio = audio_input(1025);
        let long_audio = audio_input(2051);
        let input = EmbeddingInput {
            parts: vec![
                EmbeddingPart::Image(Arc::clone(&first_image)),
                EmbeddingPart::Audio(short_audio.clone()),
                EmbeddingPart::Video(VideoInput {
                    frames: vec![
                        VideoFrame {
                            timestamp_ms: 0,
                            image: Arc::clone(&first_image),
                        },
                        VideoFrame {
                            timestamp_ms: 1000,
                            image: Arc::clone(&second_image),
                        },
                    ],
                    duration_ms: 2000,
                    audio: Some(long_audio.clone()),
                }),
            ],
            title: None,
        };
        let expected_ids = preprocessor
            .input_ids(&input, EmbeddingPurpose::Document)
            .unwrap();
        let expected_first = vision::preprocess(&first_image, 70).unwrap();
        let expected_second = vision::preprocess(&second_image, 70).unwrap();
        let expected_short = audio::preprocess(&short_audio).unwrap();
        let expected_long = audio::preprocess(&long_audio).unwrap();
        assert!(expected_short.mask.contains(&false));
        assert_ne!(expected_short.mask.len(), expected_long.mask.len());

        let prepared = std::thread::scope(|scope| {
            let handles = (0..2)
                .map(|_| {
                    let cloned = preprocessor.clone();
                    let input = &input;
                    scope.spawn(move || cloned.prepare(input, EmbeddingPurpose::Document).unwrap())
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        for prepared in prepared {
            assert!(Arc::ptr_eq(&prepared.tokenizer, &preprocessor.tokenizer));
            assert_eq!(prepared.ids, expected_ids);
            let [
                PreparedMedia::Image(image),
                PreparedMedia::Audio(short),
                PreparedMedia::Video(first),
                PreparedMedia::Video(second),
                PreparedMedia::Audio(long),
            ] = prepared.media.as_slice()
            else {
                panic!("Preparation changed the joint media encoder order");
            };
            assert_vision(image, &expected_first);
            assert_audio(short, &expected_short);
            assert_vision(first, &expected_first);
            assert_vision(second, &expected_second);
            assert_audio(long, &expected_long);
        }
    }

    #[test]
    fn context_validation_precedes_audio_preprocessing() {
        let mut preprocessor = preprocessor();
        preprocessor.options.max_tokens = 1;
        let input = EmbeddingInput {
            parts: vec![EmbeddingPart::Audio(AudioInput {
                samples: vec![f32::NAN; 500].into(),
                sample_rate: 16_000,
                channels: 1,
            })],
            title: None,
        };
        let error = preprocessor
            .prepare(&input, EmbeddingPurpose::Document)
            .err()
            .expect("Context overflow must fail before computing audio features");
        assert!(error.to_string().contains("exceeding the 1-token context"));
    }
}
