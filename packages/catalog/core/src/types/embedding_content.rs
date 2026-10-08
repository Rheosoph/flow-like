use super::node_image::NodeImage;
use flow_like_model_provider::embedding::interface::{AudioInput, EmbeddingModality};
use flow_like_types::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Decoded interleaved PCM samples. Media readers supply the original sample rate and channels.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl EmbeddingAudio {
    pub fn validate(&self) -> Result<()> {
        if self.sample_rate == 0 || self.channels == 0 || self.samples.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Audio needs samples, a sample rate, and channels"
            ));
        }
        if !self
            .samples
            .len()
            .is_multiple_of(usize::from(self.channels))
            || self.samples.iter().any(|sample| !sample.is_finite())
        {
            return Err(flow_like_types::anyhow!(
                "Audio must contain complete, finite sample frames"
            ));
        }
        Ok(())
    }

    pub fn into_audio(self) -> Result<AudioInput> {
        self.validate()?;
        let audio = AudioInput {
            samples: self.samples.into(),
            sample_rate: self.sample_rate,
            channels: self.channels,
        };
        Ok(audio)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingVideoFrame {
    pub timestamp_ms: u64,
    pub image: NodeImage,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingVideo {
    pub frames: Vec<EmbeddingVideoFrame>,
    pub duration_ms: u64,
    #[serde(default)]
    pub audio: Option<EmbeddingAudio>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EmbeddingContentPart {
    Text {
        text: String,
    },
    Image {
        image: NodeImage,
    },
    Audio(EmbeddingAudio),
    Video(EmbeddingVideo),
    /// A hosted model reads a URL, data URL, or raw base64 without decoding it locally.
    EncodedMedia {
        modality: EmbeddingModality,
        source: String,
    },
}

/// Ordered parts form one joint input and produce one vector.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingContent {
    pub parts: Vec<EmbeddingContentPart>,
    #[serde(default)]
    pub title: Option<String>,
}
