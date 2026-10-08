//! Multimodal requests use the same authenticated API transport as text embeddings.

use std::io::Cursor;

use anyhow::{Result, ensure};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};

use super::{
    hosted_input::{
        HostedEmbeddingInput, HostedEmbeddingStructuredInput, supports_internal_multimodal,
    },
    interface::{
        AudioInput, EmbeddingBatch, EmbeddingDescriptor, EmbeddingError, EmbeddingInput,
        EmbeddingLimits, EmbeddingMetric, EmbeddingModality, EmbeddingModel, EmbeddingPart,
        EmbeddingPurpose, EmbeddingRequest, EmbeddingSpace,
    },
    proxy::{EmbedResponse, ProxyEmbeddingModel},
};

pub const INTERNAL_MULTIMODAL_ADAPTER: &str = "internal_multimodal";

pub struct ProxyMultimodalEmbeddingModel {
    proxy: ProxyEmbeddingModel,
    descriptor: EmbeddingDescriptor,
}

impl ProxyMultimodalEmbeddingModel {
    pub fn new(proxy: ProxyEmbeddingModel) -> Result<Self> {
        ensure!(
            supports_internal_multimodal(&proxy.provider),
            "The multimodal embedding proxy requires Internal embeddinggemma-2"
        );
        ensure!(
            proxy.provider.vector_length == 768,
            "Internal embeddinggemma-2 returns 768-dimensional vectors"
        );
        let mut identity =
            serde_json::to_value((INTERNAL_MULTIMODAL_ADAPTER, &proxy.bit_id, &proxy.provider))?;
        identity.sort_all_objects();
        let pipeline_fingerprint = blake3::hash(&serde_json::to_vec(&identity)?)
            .to_hex()
            .to_string();
        let modalities = vec![
            EmbeddingModality::Text,
            EmbeddingModality::Image,
            EmbeddingModality::Audio,
            EmbeddingModality::Video,
        ];
        let joint_combinations = (1u8..16)
            .filter(|mask| mask.count_ones() > 1)
            .map(|mask| {
                modalities
                    .iter()
                    .enumerate()
                    .filter_map(|(index, modality)| (mask & (1 << index) != 0).then_some(*modality))
                    .collect()
            })
            .collect();
        let descriptor = EmbeddingDescriptor {
            model_id: proxy.bit_id.clone(),
            adapter: INTERNAL_MULTIMODAL_ADAPTER.into(),
            modalities,
            joint_combinations,
            purposes: vec![EmbeddingPurpose::Query, EmbeddingPurpose::Document],
            space: EmbeddingSpace {
                id: "internal:embeddinggemma-2:768".into(),
                dimensions: 768,
                normalized: false,
                metric: EmbeddingMetric::Cosine,
            },
            supported_dimensions: vec![768],
            limits: EmbeddingLimits {
                max_tokens: 8192,
                max_images_per_item: Some(1),
                max_audio_duration_ms: Some(327_000),
                ..Default::default()
            },
            // The gateway owns its preprocessing and weights; do not claim local recipe parity.
            pipeline_fingerprint,
        };
        Ok(Self { proxy, descriptor })
    }

    fn finish_response(
        &self,
        request: &EmbeddingRequest,
        response: EmbedResponse,
    ) -> Result<EmbeddingBatch, EmbeddingError> {
        let mut batch = self.descriptor.finish(request, response.embeddings)?;
        if response.usage_available && !response.usage_estimated {
            batch.usage.text_tokens = u64::try_from(response.usage.prompt_tokens).ok();
        }
        Ok(batch)
    }
}

#[async_trait]
impl EmbeddingModel for ProxyMultimodalEmbeddingModel {
    fn descriptor(&self) -> &EmbeddingDescriptor {
        &self.descriptor
    }

    fn supports_encoded_media(&self) -> bool {
        true
    }

    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingBatch, EmbeddingError> {
        self.descriptor.validate(&request)?;
        let inputs = request
            .items
            .iter()
            .map(hosted_input)
            .collect::<Result<Vec<_>, _>>()?;
        if inputs.is_empty() {
            return self.descriptor.finish(&request, Vec::new());
        }
        let embed_type = match request.purpose {
            EmbeddingPurpose::Query => "query",
            EmbeddingPurpose::Document => "document",
            purpose => return Err(EmbeddingError::UnsupportedPurpose(purpose)),
        };
        let response = self
            .proxy
            .call_hosted_api(&inputs, embed_type)
            .await
            .map_err(EmbeddingError::Backend)?;
        self.finish_response(&request, response)
    }
}

fn hosted_input(input: &EmbeddingInput) -> Result<HostedEmbeddingInput, EmbeddingError> {
    if input.title.is_some() {
        return Err(EmbeddingError::InvalidInput(
            "Hosted embedding inputs do not support a separate title; include it in the text"
                .into(),
        ));
    }
    let mut text = String::new();
    let mut ordered_text = String::new();
    let mut image = None;
    let mut audio = None;
    let mut video = None;
    let mut has_text = false;
    for part in &input.parts {
        let (modality, source) = match part {
            EmbeddingPart::Text(value) => {
                text.push_str(value);
                ordered_text.push_str(value);
                has_text = true;
                continue;
            }
            EmbeddingPart::Image(value) => {
                let mut bytes = Cursor::new(Vec::new());
                value.write_to(&mut bytes, image::ImageFormat::Png)
                    .map_err(|error| EmbeddingError::Backend(error.into()))?;
                (EmbeddingModality::Image, data_url("image/png", bytes.get_ref()))
            }
            EmbeddingPart::Audio(value) => (EmbeddingModality::Audio, audio_data_url(value)?),
            EmbeddingPart::Video(_) => return Err(EmbeddingError::InvalidInput(
                "Hosted video embeddings need the original encoded video file or URL; sampled frames cannot preserve its video timeline".into())),
            EmbeddingPart::EncodedMedia { modality, source } => (*modality, source.clone()),
        };
        let (slot, marker) = match modality {
            EmbeddingModality::Image => (&mut image, "<|image|>"),
            EmbeddingModality::Audio => (&mut audio, "<|audio|>"),
            EmbeddingModality::Video => (&mut video, "<|video|>"),
            EmbeddingModality::Text => {
                return Err(EmbeddingError::InvalidInput(
                    "Encoded media must be an image, audio, or video".into(),
                ));
            }
        };
        if slot.is_some() {
            return Err(EmbeddingError::InvalidInput(format!(
                "Hosted embedding inputs support one {modality:?} file per item"
            )));
        }
        if source.trim().is_empty() {
            return Err(EmbeddingError::InvalidInput(
                "Encoded media source is empty".into(),
            ));
        }
        *slot = Some(source);
        ordered_text.push_str(marker);
    }
    let media_count =
        usize::from(image.is_some()) + usize::from(audio.is_some()) + usize::from(video.is_some());
    if media_count == 0 {
        return Ok(HostedEmbeddingInput::Text(text));
    }
    if !has_text && media_count == 1 {
        return Ok(HostedEmbeddingInput::Structured(
            match (image, audio, video) {
                (Some(image), None, None) => HostedEmbeddingStructuredInput::Image { image },
                (None, Some(audio), None) => HostedEmbeddingStructuredInput::Audio { audio },
                (None, None, Some(video)) => HostedEmbeddingStructuredInput::Video { video },
                _ => unreachable!(),
            },
        ));
    }
    // Explicit placeholders carry the caller's desired ordering. Otherwise preserve part order.
    let has_markers = ["<|image|>", "<|audio|>", "<|video|>"]
        .iter()
        .any(|marker| text.contains(marker));
    if has_markers {
        for (marker, supplied) in [
            ("<|image|>", image.is_some()),
            ("<|audio|>", audio.is_some()),
            ("<|video|>", video.is_some()),
        ] {
            if text.matches(marker).count() != usize::from(supplied) {
                return Err(EmbeddingError::InvalidInput(format!(
                    "Hosted multimodal text must contain exactly one {marker} placeholder for each supplied file"
                )));
            }
        }
    } else {
        text = ordered_text;
    }
    Ok(HostedEmbeddingInput::Structured(
        HostedEmbeddingStructuredInput::Multimodal {
            text: Some(text),
            image,
            audio,
            video,
        },
    ))
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", STANDARD.encode(bytes))
}

fn audio_data_url(audio: &AudioInput) -> Result<String, EmbeddingError> {
    audio.validate()?;
    let data_size = audio
        .samples
        .len()
        .checked_mul(4)
        .and_then(|size| u32::try_from(size).ok())
        .filter(|size| *size <= u32::MAX - 50)
        .ok_or_else(|| EmbeddingError::LimitExceeded("Audio is too large for a WAV file".into()))?;
    let block_align = audio
        .channels
        .checked_mul(4)
        .ok_or_else(|| EmbeddingError::InvalidInput("Audio has too many channels".into()))?;
    let byte_rate = audio
        .sample_rate
        .checked_mul(u32::from(block_align))
        .ok_or_else(|| EmbeddingError::InvalidInput("Audio sample rate is too large".into()))?;
    // IEEE float WAV keeps the caller's PCM samples intact, including multichannel ordering.
    let mut wav = Vec::with_capacity(data_size as usize + 58);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(data_size + 50).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&18u32.to_le_bytes());
    wav.extend_from_slice(&3u16.to_le_bytes());
    wav.extend_from_slice(&audio.channels.to_le_bytes());
    wav.extend_from_slice(&audio.sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&32u16.to_le_bytes());
    wav.extend_from_slice(&0u16.to_le_bytes());
    wav.extend_from_slice(b"fact");
    wav.extend_from_slice(&4u32.to_le_bytes());
    wav.extend_from_slice(&(data_size / u32::from(block_align)).to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for sample in audio.samples.iter() {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(data_url("audio/wav", &wav))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        EmbeddingModelProvider, ModelProvider, Pooling, Prefix, RemoteExecutionConfig,
    };
    use std::sync::Arc;

    fn proxy() -> ProxyEmbeddingModel {
        ProxyEmbeddingModel::new(
            EmbeddingModelProvider {
                languages: vec![],
                vector_length: 768,
                input_length: 8192,
                prefix: Prefix {
                    query: String::new(),
                    paragraph: String::new(),
                },
                pooling: Pooling::Mean,
                provider: ModelProvider {
                    provider_name: "Hosted".into(),
                    model_id: Some("embeddinggemma-2".into()),
                    version: None,
                    api_surface: None,
                    params: None,
                },
                remote: Some(RemoteExecutionConfig {
                    model_id: Some("embeddinggemma-2".into()),
                    ..Default::default()
                }),
            },
            "gemma-bit".into(),
            "token".into(),
            vec![("x-flow-like-run-id".into(), "run".into())],
            "https://api.example.test".into(),
        )
    }

    #[tokio::test]
    async fn serializes_joint_media_through_the_existing_authenticated_transport() {
        let model = ProxyMultimodalEmbeddingModel::new(proxy()).unwrap();
        let signed_url = "https://media.example/video.mp4?token=a%2Fb&expires=123";
        let input = EmbeddingInput {
            title: None,
            parts: vec![
                EmbeddingPart::Text("Look ".into()),
                EmbeddingPart::Image(Arc::new(image::DynamicImage::new_rgb8(2, 3))),
                EmbeddingPart::Text(" then watch ".into()),
                EmbeddingPart::EncodedMedia {
                    modality: EmbeddingModality::Video,
                    source: signed_url.into(),
                },
                EmbeddingPart::Audio(AudioInput {
                    samples: Arc::from([0.25, -0.25]),
                    sample_rate: 16_000,
                    channels: 1,
                }),
            ],
        };
        let request = model
            .proxy
            .build_hosted_request(&[hosted_input(&input).unwrap()], "document")
            .await
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://api.example.test/api/v1/embeddings/embed"
        );
        assert_eq!(request.headers()["authorization"], "Bearer token");
        assert_eq!(request.headers()["x-flow-like-run-id"], "run");
        let body: serde_json::Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(
            body["input"][0]["text"],
            "Look <|image|> then watch <|video|><|audio|>"
        );
        assert_eq!(body["input"][0]["video"], signed_url);
        let png = STANDARD
            .decode(
                body["input"][0]["image"]
                    .as_str()
                    .unwrap()
                    .split_once(',')
                    .unwrap()
                    .1,
            )
            .unwrap();
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap();
        assert_eq!((image.width(), image.height()), (2, 3));
        let wav = STANDARD
            .decode(
                body["input"][0]["audio"]
                    .as_str()
                    .unwrap()
                    .split_once(',')
                    .unwrap()
                    .1,
            )
            .unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize + 8,
            wav.len()
        );
        assert_eq!(&wav[58..62], &0.25f32.to_le_bytes());
        assert!(model.supports_encoded_media());
        assert_eq!(
            model.descriptor().limits.max_audio_duration_ms,
            Some(327_000)
        );
    }

    #[test]
    fn retains_explicit_placeholder_order_and_rejects_duplicate_media() {
        let input = EmbeddingInput {
            title: None,
            parts: vec![
                EmbeddingPart::Text("Hear <|audio|> then see <|image|>".into()),
                EmbeddingPart::EncodedMedia {
                    modality: EmbeddingModality::Image,
                    source: "AAAA".into(),
                },
                EmbeddingPart::EncodedMedia {
                    modality: EmbeddingModality::Audio,
                    source: "BBBB".into(),
                },
            ],
        };
        let json = serde_json::to_value(hosted_input(&input).unwrap()).unwrap();
        assert_eq!(json["text"], "Hear <|audio|> then see <|image|>");
        let mut duplicate = input.clone();
        duplicate.parts.push(input.parts[2].clone());
        assert!(hosted_input(&duplicate).is_err());
        let mut missing = input;
        missing.parts.pop();
        assert!(hosted_input(&missing).is_err());
    }

    #[test]
    fn sends_original_video_and_keeps_configuration_identity() {
        let video = EmbeddingInput {
            title: None,
            parts: vec![EmbeddingPart::EncodedMedia {
                modality: EmbeddingModality::Video,
                source: "data:video/mp4;base64,AAAA".into(),
            }],
        };
        assert_eq!(
            serde_json::to_value(hosted_input(&video).unwrap()).unwrap(),
            serde_json::json!({"type":"video", "video":"data:video/mp4;base64,AAAA"})
        );
        let initial = ProxyMultimodalEmbeddingModel::new(proxy()).unwrap();
        let mut changed = proxy();
        changed.provider.prefix.query = "different task prefix".into();
        let changed = ProxyMultimodalEmbeddingModel::new(changed).unwrap();
        assert_ne!(
            initial.descriptor().pipeline_fingerprint,
            changed.descriptor().pipeline_fingerprint
        );
        let mut other_provider = proxy();
        other_provider
            .provider
            .remote
            .as_mut()
            .unwrap()
            .implementation = Some(crate::provider::RemoteEmbeddingProvider::OpenAICompatible);
        assert!(ProxyMultimodalEmbeddingModel::new(other_provider).is_err());
        let mut first = proxy();
        first.provider.provider.params = Some(std::collections::HashMap::from([
            ("a".into(), serde_json::json!(1)),
            ("b".into(), serde_json::json!(2)),
        ]));
        let mut second = proxy();
        second.provider.provider.params = Some(std::collections::HashMap::from([
            ("b".into(), serde_json::json!(2)),
            ("a".into(), serde_json::json!(1)),
        ]));
        assert_eq!(
            ProxyMultimodalEmbeddingModel::new(first)
                .unwrap()
                .descriptor()
                .pipeline_fingerprint,
            ProxyMultimodalEmbeddingModel::new(second)
                .unwrap()
                .descriptor()
                .pipeline_fingerprint
        );
    }

    #[tokio::test]
    async fn rejects_unsupported_shape_before_network_and_validates_gateway_vectors() {
        let model = ProxyMultimodalEmbeddingModel::new(proxy()).unwrap();
        let mut request = EmbeddingRequest::texts(["query".into()], EmbeddingPurpose::Query);
        request.options.dimensions = Some(256);
        assert!(matches!(
            model.embed(request.clone()).await,
            Err(EmbeddingError::UnsupportedDimensions(256))
        ));
        request.options.dimensions = None;
        assert!(
            model
                .descriptor()
                .finish(&request, vec![vec![0.0; 512]])
                .is_err()
        );
        assert!(
            model
                .descriptor()
                .finish(&request, vec![vec![f32::NAN; 768]])
                .is_err()
        );
        assert!(
            model
                .descriptor()
                .finish(&request, vec![vec![0.0; 768]])
                .is_ok()
        );
        request.items[0].parts = vec![EmbeddingPart::Audio(AudioInput {
            samples: Arc::from(vec![0.0; 328]),
            sample_rate: 1,
            channels: 1,
        })];
        assert!(matches!(
            model.embed(request).await,
            Err(EmbeddingError::LimitExceeded(_))
        ));
    }

    #[test]
    fn only_reports_measured_gateway_token_usage() {
        let model = ProxyMultimodalEmbeddingModel::new(proxy()).unwrap();
        let request = EmbeddingRequest::texts(["query".into()], EmbeddingPurpose::Query);
        for (available, estimated, tokens) in [
            (Some(true), Some(false), Some(42)),
            (Some(false), Some(false), None),
            (Some(true), Some(true), None),
            (Some(false), Some(true), None),
            (None, None, None),
        ] {
            let mut wire = serde_json::json!({
                "model": "gemma-bit", "embeddings": [vec![0.0; 768]],
                "usage": {"prompt_tokens": 42, "total_tokens": 42}
            });
            if let Some(value) = available {
                wire["usage_available"] = value.into();
            }
            if let Some(value) = estimated {
                wire["usage_estimated"] = value.into();
            }
            let response: EmbedResponse = serde_json::from_value(wire).unwrap();
            assert_eq!(
                model
                    .finish_response(&request, response)
                    .unwrap()
                    .usage
                    .text_tokens,
                tokens
            );
        }
    }
}
