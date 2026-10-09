use crate::{BackendChoice, Error, PretrainedWeightsMetadata, Result, TensorData};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sam2Variant {
    HieraTiny,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct Sam2Config {
    pub variant: Sam2Variant,
}
impl Sam2Config {
    pub const IMAGE_SIZE: usize = 1024;
    pub const MASK_SIZE: usize = 256;

    pub fn validate(&self) -> Result<()> {
        Ok(())
    }

    pub fn validate_input_shape(&self, shape: &[usize]) -> Result<()> {
        self.validate()?;
        if shape.len() != 4 || shape[0] == 0 || shape[1..] != [3, 1024, 1024] {
            return Err(Error::Invalid(
                "SAM 2.1 Hiera requires NCHW RGB images resized to 1024 by 1024".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct Sam2Point {
    pub x: f32,
    pub y: f32,
    /// Zero denotes background and one denotes foreground.
    pub label: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct Sam2Prompt {
    #[serde(default)]
    pub points: Vec<Sam2Point>,
    /// Pixel coordinates [left, top, right, bottom] in the resized image.
    #[serde(default)]
    pub bbox: Option<[f32; 4]>,
    /// Previous low-resolution mask logits with shape [1, 256, 256].
    #[serde(default)]
    pub mask: Option<TensorData>,
}
impl Sam2Prompt {
    pub fn validate(&self) -> Result<()> {
        if self.points.len() > 1024
            || self.points.iter().any(|point| {
                !point.x.is_finite()
                    || !point.y.is_finite()
                    || !(0.0..1024.0).contains(&point.x)
                    || !(0.0..1024.0).contains(&point.y)
                    || !matches!(point.label, 0 | 1)
            })
            || self.bbox.is_some_and(|bounds| {
                bounds
                    .iter()
                    .any(|value| !value.is_finite() || !(0.0..=1024.0).contains(value))
                    || bounds[0] >= bounds[2]
                    || bounds[1] >= bounds[3]
            })
        {
            return Err(Error::Invalid(
                "Invalid SAM image prompt coordinates or labels".into(),
            ));
        }
        if let Some(mask) = &self.mask {
            mask.validate()?;
            if mask.shape != [1, 256, 256] {
                return Err(Error::Invalid(
                    "SAM mask prompts require shape [1, 256, 256]".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct Sam2TrainingConfig {
    pub model: Sam2Config,
    pub backend: BackendChoice,
    pub epochs: usize,
    pub batch_size: usize,
    pub learning_rate: f64,
    pub seed: u64,
    pub gradient_clip: f32,
    #[serde(default = "freeze_encoder_by_default")]
    pub freeze_image_encoder: bool,
}
fn freeze_encoder_by_default() -> bool {
    true
}
impl Sam2TrainingConfig {
    pub fn validate(&self) -> Result<()> {
        self.model.validate()?;
        if self.epochs == 0
            || self.epochs > 1_000_000
            || self.batch_size == 0
            || self.batch_size > 1024
            || !self.learning_rate.is_finite()
            || self.learning_rate <= 0.0
            || self.learning_rate > 1.0
            || !self.gradient_clip.is_finite()
            || self.gradient_clip <= 0.0
            || self.gradient_clip > 1_000_000.0
        {
            return Err(Error::Invalid(
                "SAM training settings exceed their supported limits".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct Sam2TrainingDataset {
    /// NCHW RGB images resized to 1024 and normalized with ImageNet mean and standard deviation.
    pub images: TensorData,
    pub prompts: Vec<Sam2Prompt>,
    /// Binary foreground masks with shape [batch, 1, 256, 256].
    pub masks: TensorData,
}
impl Sam2TrainingDataset {
    pub fn validate(&self, config: &Sam2Config) -> Result<()> {
        self.images.validate()?;
        self.masks.validate()?;
        config.validate_input_shape(&self.images.shape)?;
        let count = self.images.shape[0];
        if self.prompts.len() != count
            || self.masks.shape != [count, 1, 256, 256]
            || self
                .masks
                .values
                .iter()
                .any(|value| !matches!(*value, 0.0 | 1.0))
        {
            return Err(Error::Invalid(
                "SAM training needs one prompt and binary 256 by 256 mask per image".into(),
            ));
        }
        for prompt in &self.prompts {
            prompt.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct Sam2WeightsProvenance {
    pub metadata: PretrainedWeightsMetadata,
    pub format: String,
    pub sha256: String,
}
impl Sam2WeightsProvenance {
    pub fn validate(&self) -> Result<()> {
        self.metadata.validate()?;
        if self.format.trim().is_empty()
            || self.format.len() > 256
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self
                .metadata
                .expected_sha256
                .as_ref()
                .is_some_and(|expected| !expected.eq_ignore_ascii_case(&self.sha256))
        {
            return Err(Error::Invalid("Invalid SAM weight provenance".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct Sam2ImportedModelInfo {
    pub config: Sam2Config,
    pub provenance: Sam2WeightsProvenance,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct Sam2Prediction {
    pub mask_logits: TensorData,
    pub iou: TensorData,
    pub object_score_logits: TensorData,
}
